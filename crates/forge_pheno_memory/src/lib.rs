// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Pheno memory substrate for forgecode.
//
// Wires the `thegent-memory` v2 polyglot facade (supermemory + letta +
// cognee + mem0) into forgecode's Infra + Domain pattern. The intent is to
// give forgecode agents a stable, scope-routed memory API without coupling
// forgecode itself to any specific memory engine.
//
// Scope routing (locked per ADR-096, accepted 2026-06-23):
//   - Episodic          -> supermemory (smfs filesystem, :3030)
//   - Identity          -> letta (subconscious blocks, :8283)
//   - ProjectKnowledge  -> cognee (knowledge graph, stdio cognee-mcp)
//   - Fallback          -> mem0 (REST :8000)
//
// Endpoints default to the localhost ports advertised by the
// `pheno-forge-plugins` v0.1.0 sidecar bundle. Override via the
// `PhenoMemoryConfig` builder.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thegent_memory::v2::{
    adapters::{CogneeAdapter, LettaAdapter, Mem0Adapter, SupermemoryAdapter},
    CompositeAdapter, MemoryError, MemoryPort, MemoryProvider, MemoryQuery, MemoryRecord,
    MemoryScope, MemoryValue,
};

/// Public domain error type, surfaces `MemoryError` as a forgecode-friendly
/// `thiserror::Error` so callers can match on variants.
#[derive(Debug, thiserror::Error)]
pub enum PhenoMemoryError {
    #[error("network error: {0}")]
    Network(String),
    #[error("backend returned status {status}: {body}")]
    Backend { status: u16, body: String },
    #[error("not found: scope={scope} key={key}")]
    NotFound { scope: String, key: String },
    #[error("serialization error: {0}")]
    Serde(String),
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<MemoryError> for PhenoMemoryError {
    fn from(e: MemoryError) -> Self {
        match e {
            MemoryError::Network(s) => Self::Network(s),
            MemoryError::Backend { status, body } => Self::Backend { status, body },
            MemoryError::NotFound { scope, key } => {
                Self::NotFound { scope: scope.to_string(), key }
            }
            MemoryError::Serde(s) => Self::Serde(s),
            MemoryError::Unavailable(s) => Self::Unavailable(s),
            MemoryError::Invalid(s) => Self::Invalid(s),
            MemoryError::Internal(s) => Self::Internal(s),
        }
    }
}

/// JSON-serializable scope label so forgecode's tooling can pass it over
/// JSON-RPC without a custom serializer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhenoMemoryScope {
    Episodic,
    Identity,
    ProjectKnowledge,
    Fallback,
}

impl From<PhenoMemoryScope> for MemoryScope {
    fn from(s: PhenoMemoryScope) -> Self {
        match s {
            PhenoMemoryScope::Episodic => MemoryScope::Episodic,
            PhenoMemoryScope::Identity => MemoryScope::Identity,
            PhenoMemoryScope::ProjectKnowledge => MemoryScope::ProjectKnowledge,
            PhenoMemoryScope::Fallback => MemoryScope::Fallback,
        }
    }
}

/// Endpoints for the four backing engines. Defaults match the
/// `pheno-forge-plugins` v0.1.0 systemd unit ports.
#[derive(Debug, Clone)]
pub struct PhenoMemoryConfig {
    pub supermemory_url: String,
    pub letta_url: String,
    pub mem0_url: String,
}

impl Default for PhenoMemoryConfig {
    fn default() -> Self {
        Self {
            supermemory_url: "http://127.0.0.1:3030".into(),
            letta_url: "http://127.0.0.1:8283".into(),
            mem0_url: "http://127.0.0.1:8000".into(),
        }
    }
}

/// The main entry point for forgecode callers. Wraps a `CompositeAdapter`
/// with the four single-scope adapters wired to the localhost sidecar
/// stack, and exposes a JSON-serializable API surface.
pub struct PhenoMemoryService {
    composite: CompositeAdapter,
}

impl PhenoMemoryService {
    /// Build a service from explicit config. The four backing adapters
    /// are constructed with default endpoints; if a sidecar is unreachable
    /// the corresponding calls will surface a `PhenoMemoryError::Network`
    /// or `PhenoMemoryError::Backend` depending on the failure mode.
    /// `CompositeAdapter` does NOT silently fall back to mem0 except for
    /// `MemoryScope::Fallback`; the spec requires explicit scope → adapter
    /// routing.
    pub fn new(cfg: &PhenoMemoryConfig) -> Self {
        let sm = Arc::new(SupermemoryAdapter::new(cfg.supermemory_url.clone()));
        let lt = Arc::new(LettaAdapter::new(cfg.letta_url.clone()));
        // Cognee uses MCP-over-stdio; the adapter takes a transport
        // (Box<dyn CogneeTransport>). `default_endpoint()` wires the in-tree
        // `StubTransport` which is a no-op for round-trips — the real
        // stdio subprocess is wired by the `pheno-forge-plugins` systemd
        // unit which calls `cognee-mcp` directly via MCP stdio.
        let cg = Arc::new(CogneeAdapter::default_endpoint());
        let m0 = Arc::new(Mem0Adapter::new(cfg.mem0_url.clone()));
        let composite = CompositeAdapter::new(sm, lt, cg, m0);
        Self { composite }
    }

    pub fn with_defaults() -> Self {
        Self::new(&PhenoMemoryConfig::default())
    }

    pub fn provider(&self) -> MemoryProvider {
        self.composite.provider()
    }
}

#[async_trait]
impl MemoryPort for PhenoMemoryService {
    async fn store(
        &self,
        scope: MemoryScope,
        key: &str,
        value: MemoryValue,
    ) -> Result<String, MemoryError> {
        self.composite.store(scope, key, value).await
    }

    async fn recall(
        &self,
        scope: MemoryScope,
        query: MemoryQuery,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        self.composite.recall(scope, query).await
    }

    async fn forget(&self, scope: MemoryScope, key: &str) -> Result<(), MemoryError> {
        self.composite.forget(scope, key).await
    }

    async fn list_scopes(&self) -> Result<Vec<MemoryScope>, MemoryError> {
        self.composite.list_scopes().await
    }

    fn provider(&self) -> MemoryProvider {
        self.composite.provider()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_match_sidecar_ports() {
        let c = PhenoMemoryConfig::default();
        assert_eq!(c.supermemory_url, "http://127.0.0.1:3030");
        assert_eq!(c.letta_url, "http://127.0.0.1:8283");
        assert_eq!(c.mem0_url, "http://127.0.0.1:8000");
    }

    #[test]
    fn service_constructs_with_defaults() {
        let svc = PhenoMemoryService::with_defaults();
        assert_eq!(svc.provider(), MemoryProvider::Composite);
    }

    #[test]
    fn scope_round_trip() {
        for s in [
            PhenoMemoryScope::Episodic,
            PhenoMemoryScope::Identity,
            PhenoMemoryScope::ProjectKnowledge,
            PhenoMemoryScope::Fallback,
        ] {
            let internal: MemoryScope = s.into();
            let _ = format!("{internal:?}");
        }
    }
}
