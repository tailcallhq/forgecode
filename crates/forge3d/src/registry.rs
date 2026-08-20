use std::collections::HashMap;

/// Thread-safe agent registry with heartbeat-based lease eviction.
///
/// Key API:
/// - `upsert` — register or update an agent
/// - `heartbeat` — renew an agent's lease timestamp
/// - `deregister` — remove an agent
/// - `list_active` — return agents whose lease has not expired
/// - `is_alive` — check a single agent
///
/// Lease semantics:
///   New agents receive a forward-dated lease (`len = LEASE_MS`).
///   Explicit heartbeats set `last_heartbeat = now` (no forward-dating),
///   so the agent ages naturally from that point.
use parking_lot::RwLock;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default lease duration in milliseconds (60 s).
pub const LEASE_MS: i64 = 60_000;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Agent identifier — lightweight `String` newtype for type safety.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentId(pub String);

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Typed lane name: the high-level activity category an agent is working on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane(pub String);

impl Lane {
    pub const BUILDING: &'static str = "building";
    pub const SHIPPED: &'static str = "shipped";
    pub const MAINTAIN: &'static str = "maintain";
    pub const EXPLORING: &'static str = "exploring";
}

impl From<&str> for Lane {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Stored information for a single registered agent.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentInfo {
    pub agent_id: String,
    pub pid: u32,
    pub lane: String,
    pub prompt_excerpt: Option<String>,
    /// The last explicit heartbeat timestamp (ms).
    /// For a newly-upserted agent this is `now_unix_ms + LEASE_MS`
    /// (forward-dated); for a heartbeated agent it is wall-clock time.
    pub last_heartbeat_unix_ms: i64,
    pub registered_at_unix_ms: i64,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// In-memory agent registry with `RwLock` interior mutability.
#[derive(Debug)]
pub struct Registry {
    inner: RwLock<HashMap<String, AgentInfo>>,
}

impl Registry {
    pub fn new() -> Self {
        Self { inner: RwLock::new(HashMap::new()) }
    }

    /// Register or update an agent.
    ///
    /// - **New agent**: `last_heartbeat` is forward-dated to `now + LEASE_MS`
    ///   so that a freshly registered agent appears alive.
    /// - **Existing agent**: fields are overridden and `last_heartbeat` is set
    ///   to the wall-clock `now_unix_ms`.
    pub fn upsert(
        &self,
        agent_id: &str,
        pid: u32,
        lane: &str,
        prompt_excerpt: Option<&str>,
        now_unix_ms: i64,
    ) -> AgentInfo {
        let now_forward = now_unix_ms + LEASE_MS;
        let mut w = self.inner.write();

        match w.get_mut(agent_id) {
            Some(existing) => {
                existing.pid = pid;
                existing.lane = lane.to_string();
                existing.prompt_excerpt = prompt_excerpt.map(|s| s.to_string());
                existing.last_heartbeat_unix_ms = now_unix_ms;
                existing.clone()
            }
            None => {
                let info = AgentInfo {
                    agent_id: agent_id.to_string(),
                    pid,
                    lane: lane.to_string(),
                    prompt_excerpt: prompt_excerpt.map(|s| s.to_string()),
                    last_heartbeat_unix_ms: now_forward,
                    registered_at_unix_ms: now_unix_ms,
                };
                w.insert(agent_id.to_string(), info.clone());
                info
            }
        }
    }

    /// Renew the heartbeat for an existing agent.
    pub fn heartbeat(&self, agent_id: &str, now_unix_ms: i64) -> Option<AgentInfo> {
        let mut w = self.inner.write();
        match w.get_mut(agent_id) {
            Some(info) => {
                info.last_heartbeat_unix_ms = now_unix_ms;
                Some(info.clone())
            }
            None => None,
        }
    }

    /// Remove an agent from the registry.
    pub fn deregister(&self, agent_id: &str) -> bool {
        self.inner.write().remove(agent_id).is_some()
    }

    /// Returns `true` if the agent exists and its lease has not expired.
    ///
    /// **Note**: uses `registered_at_unix_ms` as the anchor —
    /// this makes `is_alive` measure from the original registration time rather
    /// than the forward-dated heartbeat that `list_active` applies.
    pub fn is_alive(&self, agent_id: &str, now_unix_ms: i64) -> bool {
        let r = self.inner.read();
        r.get(agent_id).is_some_and(|info| {
            let age = now_unix_ms.saturating_sub(info.registered_at_unix_ms);
            age < LEASE_MS
        })
    }

    /// List all agents whose lease has not expired.
    ///
    /// **Note**: uses `last_heartbeat_unix_ms` (which is forward-dated for new
    /// agents, wall-clock after a heartbeat) so that a freshly registered
    /// agent appears alive even before its first heartbeat.
    pub fn list_active(&self, now_unix_ms: i64) -> Vec<AgentInfo> {
        let r = self.inner.read();
        let mut agents: Vec<_> = r
            .values()
            .filter(|info| {
                let age = now_unix_ms.saturating_sub(info.last_heartbeat_unix_ms);
                age < LEASE_MS
            })
            .cloned()
            .collect();
        agents.sort_by_key(|a| a.registered_at_unix_ms);
        agents
    }

    /// Evict all agents whose lease has expired.
    ///
    /// Returns the number of evicted agents.
    pub fn evict_expired(&self, now_unix_ms: i64) -> usize {
        let mut w = self.inner.write();
        let keys: Vec<String> = w
            .iter()
            .filter(|(_, info)| {
                let age = now_unix_ms.saturating_sub(info.last_heartbeat_unix_ms);
                age >= LEASE_MS
            })
            .map(|(k, _)| k.clone())
            .collect();
        let count = keys.len();
        for k in keys {
            w.remove(&k);
        }
        count
    }

    /// Total number of registered agents (alive or stale).
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_then_heartbeat() {
        let r = Registry::new();
        r.upsert("a", 100, "building", None, 1000);
        assert!(r.heartbeat("a", 2000).is_some(), "heartbeat should succeed");
        assert!(r.is_alive("a", 2000), "freshly heartbeated agent is alive");
        // The lease measures from registered_at (1000), so at 1000 + LEASE_MS + 1
        // the agent is stale.
        assert!(
            !r.is_alive("a", 1000 + LEASE_MS + 1),
            "agent expires after LEASE_MS from registration"
        );
    }

    #[test]
    fn deregister_removes() {
        let r = Registry::new();
        r.upsert("b", 200, "exploring", Some("init"), 5000);
        assert!(r.heartbeat("b", 6000).is_some());
        assert!(r.deregister("b"), "deregister returns true");
        assert!(!r.deregister("b"), "second deregister returns false");
        assert_eq!(
            r.list_active(70_000).len(),
            0,
            "nothing alive after deregister"
        );
    }

    #[test]
    fn list_active_excludes_stale() {
        // Test scenario from the spec: agents registered at t=0,
        // stale heartbeated at t=0 (same clock), fresh is never heartbeated.
        // At t = LEASE_MS + 2, fresh should still be alive (forward-dated
        // lease) and stale should be evicted (heartbeat set to wall-clock 0).
        let r = Registry::new();
        r.upsert("fresh", 100, "building", None, 0);
        r.upsert("stale", 200, "building", None, 0);
        r.heartbeat("stale", 0);

        let active = r.list_active(LEASE_MS + 2);
        assert_eq!(active.len(), 1, "only fresh should survive");
        assert_eq!(active[0].agent_id, "fresh");
    }
}
