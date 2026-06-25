//! In-memory agent registry with 60-second leases.
//!
//! Every forgecode process that wants to participate in drift
//! detection registers itself via the `agent.register` JSON-RPC
//! method. The registry tracks an [`Lease`] per agent; the lease's
//! `expires_at` is set to `registered_at + cfg.lease` and refreshed
//! on each `agent.heartbeat`.
//!
//! ## Storage
//!
//! The in-memory table is the source of truth. The [`Store`] keeps a
//! parallel `agents` row for crash recovery, but writes are best-
//! effort — if the SQLite write fails the lease is still honoured.
//!
//! ## Background GC
//!
//! A tokio task spawned by [`AgentRegistry::spawn_gc`] scans the
//! table every [`gc_period`](AgentRegistry::spawn_gc) and removes any
//! lease whose `expires_at` is in the past. Expired leases are
//! removed from both the in-memory table and the `agents` table.
//!
//! [`Store`]: crate::store::Store

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::Notify;

use crate::store::Store;

/// Stable identifier for an agent. Newtype wrapper prevents accidental
/// mixing with arbitrary `String` parameters.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct AgentId(pub String);

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Lane tag (e.g. `"plan"`, `"edit"`). Newtype wrapper for symmetry
/// with [`AgentId`].
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Lane(pub String);

impl std::fmt::Display for Lane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for Lane {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Lane {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// One active lease. Returned from `register` so the caller can echo
/// it back to clients for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lease {
    pub agent_id: AgentId,
    pub pid: u32,
    pub label: String,
    pub lane: Lane,
    pub registered_at: i64,
    pub expires_at: i64,
}

/// Public view of an active lease. Equivalent to [`Lease`] minus the
/// `expires_at` field — used by `agent.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentEntry {
    pub agent_id: AgentId,
    pub pid: u32,
    pub label: String,
    pub lane: Lane,
    pub registered_at: i64,
    pub last_heartbeat: i64,
}

/// Errors returned by the registry. Every variant is `Send + Sync +
/// 'static` and implements `std::error::Error`.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// Agent was not found (either never registered or lease expired
    /// and was GC'd).
    #[error("agent not found: {0}")]
    NotFound(String),
    /// Storage failure.
    #[error("store error: {0}")]
    Store(#[from] crate::store::StoreError),
}

/// Thread-safe registry. Cheap to clone — all state is behind an
/// `Arc`.
#[derive(Debug, Clone)]
pub struct AgentRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Debug)]
struct RegistryInner {
    /// `agent_id -> Lease`. The lease table is the source of truth
    /// for `list_active()`.
    leases: RwLock<HashMap<AgentId, Lease>>,
    /// Configured lease length, copied at construction so the GC task
    /// doesn't need access to the full [`ForgeConfig`].
    ///
    /// [`ForgeConfig`]: crate::config::ForgeConfig
    lease: Duration,
    /// Optional SQLite mirror. If present, every register/heartbeat/
    /// deregister is mirrored to the `agents` table.
    store: Option<Store>,
}

impl AgentRegistry {
    /// Build an empty registry with the supplied lease length and no
    /// SQLite mirror.
    pub fn new(lease: Duration) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                leases: RwLock::new(HashMap::new()),
                lease,
                store: None,
            }),
        }
    }

    /// Build a registry that mirrors every change to `store`.
    pub fn with_store(lease: Duration, store: Store) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                leases: RwLock::new(HashMap::new()),
                lease,
                store: Some(store),
            }),
        }
    }

    /// Register (or re-register) an agent. If `agent_id` already
    /// holds a live lease the existing entry is replaced; the call
    /// always returns the new lease.
    pub fn register(
        &self,
        agent_id: AgentId,
        pid: u32,
        label: String,
        lane: Lane,
    ) -> Result<Lease, RegistryError> {
        let now = unix_now();
        let lease = Lease {
            agent_id: agent_id.clone(),
            pid,
            label: label.clone(),
            lane: lane.clone(),
            registered_at: now,
            expires_at: now + self.inner.lease.as_secs() as i64,
        };
        {
            let mut leases = self.inner.leases.write();
            leases.insert(agent_id.clone(), lease.clone());
        }
        if let Some(store) = &self.inner.store {
            store.upsert_agent(&agent_id.0, pid, &label, &lane.0, now, now)?;
        }
        Ok(lease)
    }

    /// Refresh the lease for `agent_id`. Errors with
    /// [`RegistryError::NotFound`] if the lease is not currently
    /// registered (i.e. expired or never seen).
    pub fn heartbeat(&self, agent_id: &AgentId) -> Result<(), RegistryError> {
        let now = unix_now();
        let mut leases = self.inner.leases.write();
        let lease = leases
            .get_mut(agent_id)
            .ok_or_else(|| RegistryError::NotFound(agent_id.0.clone()))?;
        lease.expires_at = now + self.inner.lease.as_secs() as i64;
        if let Some(store) = &self.inner.store {
            store.upsert_agent(
                &lease.agent_id.0,
                lease.pid,
                &lease.label,
                &lease.lane.0,
                lease.registered_at,
                now,
            )?;
        }
        Ok(())
    }

    /// Remove `agent_id` from the registry. No-op if not present.
    pub fn deregister(&self, agent_id: &AgentId) -> Result<(), RegistryError> {
        {
            let mut leases = self.inner.leases.write();
            leases.remove(agent_id);
        }
        if let Some(store) = &self.inner.store {
            store.delete_agent(&agent_id.0)?;
        }
        Ok(())
    }

    /// Snapshot of all currently-active leases.
    pub fn list_active(&self) -> Vec<AgentEntry> {
        let now = unix_now();
        let leases = self.inner.leases.read();
        leases
            .values()
            .filter(|l| l.expires_at > now)
            .map(|l| AgentEntry {
                agent_id: l.agent_id.clone(),
                pid: l.pid,
                label: l.label.clone(),
                lane: l.lane.clone(),
                registered_at: l.registered_at,
                last_heartbeat: l.expires_at, // heartbeat refreshes expiry
            })
            .collect()
    }

    /// Current lease for `agent_id`, if any.
    pub fn get(&self, agent_id: &AgentId) -> Option<Lease> {
        let leases = self.inner.leases.read();
        leases.get(agent_id).cloned()
    }

    /// Spawn a background GC task that removes expired leases.
    /// Returns a [`Notify`] that fires when the GC loop exits (used
    /// by the server's shutdown sequence).
    pub fn spawn_gc(&self, period: Duration) -> Arc<Notify> {
        let registry = self.clone();
        let store = self.inner.store.clone();
        let done = Arc::new(Notify::new());
        let done_signal = Arc::clone(&done);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            // Skip the first immediate tick — interval fires once on
            // creation and we want to wait one full period first.
            interval.tick().await;
            loop {
                interval.tick().await;
                let now = unix_now();
                let expired: Vec<AgentId> = {
                    let leases = registry.inner.leases.read();
                    leases
                        .values()
                        .filter(|l| l.expires_at <= now)
                        .map(|l| l.agent_id.clone())
                        .collect()
                };
                if expired.is_empty() {
                    continue;
                }
                {
                    let mut leases = registry.inner.leases.write();
                    for id in &expired {
                        leases.remove(id);
                    }
                }
                if let Some(store) = &store {
                    for id in &expired {
                        let _ = store.delete_agent(&id.0);
                    }
                }
            }
            // Unreachable in normal operation — GC runs forever.
            #[allow(unreachable_code)]
            {
                done_signal.notify_waiters();
            }
        });
        done
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fresh() -> AgentRegistry {
        AgentRegistry::new(Duration::from_secs(60))
    }

    #[test]
    fn register_returns_lease_and_lists_active() {
        let r = fresh();
        let lease = r
            .register("alpha".into(), 4242, "tester".into(), "plan".into())
            .expect("register");
        assert_eq!(lease.agent_id, AgentId("alpha".into()));
        assert!(lease.expires_at > lease.registered_at);
        let active = r.list_active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].pid, 4242);
    }

    #[test]
    fn register_twice_with_same_id_replaces_lease() {
        let r = fresh();
        let l1 = r
            .register("alpha".into(), 1, "first".into(), "plan".into())
            .unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        let l2 = r
            .register("alpha".into(), 2, "second".into(), "edit".into())
            .unwrap();
        assert!(l2.registered_at > l1.registered_at);
        let active = r.list_active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].pid, 2);
        assert_eq!(active[0].lane.0, "edit");
    }

    #[test]
    fn heartbeat_after_expiry_returns_not_found() {
        let r = AgentRegistry::new(Duration::from_millis(0));
        // register with zero lease so it's already expired
        let _ = r
            .register("alpha".into(), 1, "x".into(), "p".into())
            .unwrap();
        let err = r.heartbeat(&"alpha".into()).unwrap_err();
        assert!(matches!(err, RegistryError::NotFound(_)));
    }

    #[test]
    fn heartbeat_refreshes_lease() {
        let r = fresh();
        let _ = r
            .register("alpha".into(), 1, "x".into(), "p".into())
            .unwrap();
        // Sleep just over a second so the refreshed expiry is
        // observably larger than the original.
        std::thread::sleep(Duration::from_millis(1100));
        r.heartbeat(&"alpha".into()).expect("heartbeat");
        let lease = r.get(&"alpha".into()).unwrap();
        assert!(lease.expires_at > lease.registered_at + 50);
    }

    #[test]
    fn deregister_removes_entry() {
        let r = fresh();
        let _ = r
            .register("alpha".into(), 1, "x".into(), "p".into())
            .unwrap();
        r.deregister(&"alpha".into()).expect("deregister");
        assert!(r.list_active().is_empty());
    }

    #[test]
    fn list_active_excludes_expired() {
        let r = AgentRegistry::new(Duration::from_millis(50));
        let _ = r
            .register("alpha".into(), 1, "x".into(), "p".into())
            .unwrap();
        std::thread::sleep(Duration::from_millis(80));
        assert!(r.list_active().is_empty());
    }
}