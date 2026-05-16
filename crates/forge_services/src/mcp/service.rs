use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use forge_app::domain::{
    McpConfig, McpServerConfig, McpServers, ServerName, ToolCallFull, ToolDefinition, ToolName,
    ToolOutput,
};
use forge_app::{
    EnvironmentInfra, KVStore, McpClientInfra, McpConfigManager, McpServerInfra, McpService,
};
use tokio::sync::{Mutex, RwLock};

use crate::mcp::tool::McpExecutor;

fn generate_mcp_tool_name(server_name: &ServerName, tool_name: &ToolName) -> ToolName {
    let sanitized_server_name = ToolName::sanitized(server_name.to_string().as_str());
    let sanitized_tool_name = tool_name.clone().into_sanitized();

    ToolName::new(format!(
        "mcp_{sanitized_server_name}_tool_{sanitized_tool_name}"
    ))
}

#[derive(Clone)]
pub struct ForgeMcpService<M, I, C> {
    tools: Arc<RwLock<HashMap<ToolName, ToolHolder<McpExecutor<C>>>>>,
    failed_servers: Arc<RwLock<HashMap<ServerName, String>>>,
    previous_config_hash: Arc<Mutex<u64>>,
    init_lock: Arc<Mutex<()>>,
    manager: Arc<M>,
    infra: Arc<I>,
}

#[derive(Clone)]
struct ToolHolder<T> {
    definition: ToolDefinition,
    executable: T,
    server_name: String,
}

impl<M, I, C> ForgeMcpService<M, I, C>
where
    M: McpConfigManager,
    I: McpServerInfra + KVStore + EnvironmentInfra,
    C: McpClientInfra + Clone,
    C: From<<I as McpServerInfra>::Client>,
{
    pub fn new(manager: Arc<M>, infra: Arc<I>) -> Self {
        Self {
            tools: Default::default(),
            failed_servers: Default::default(),
            previous_config_hash: Arc::new(Mutex::new(Default::default())),
            init_lock: Arc::new(Mutex::new(())),
            manager,
            infra,
        }
    }

    async fn is_config_modified(&self, config: &McpConfig) -> bool {
        *self.previous_config_hash.lock().await != config.cache_key()
    }

    async fn insert_clients(&self, server_name: &ServerName, client: Arc<C>) -> anyhow::Result<()> {
        let tools = client.list().await?;

        let mut tool_map = self.tools.write().await;

        for mut tool in tools.into_iter() {
            let actual_name = tool.name.clone();
            let server = McpExecutor::new(actual_name, client.clone())?;
            let generated_name = generate_mcp_tool_name(server_name, &tool.name);

            tool.name = generated_name.clone();

            tool_map.insert(
                generated_name,
                ToolHolder {
                    definition: tool,
                    executable: server,
                    server_name: server_name.to_string(),
                },
            );
        }

        Ok(())
    }

    async fn connect(
        &self,
        server_name: &ServerName,
        config: McpServerConfig,
    ) -> anyhow::Result<()> {
        let env_vars = self.infra.get_env_vars();
        let environment = self.infra.get_environment();
        let client = self.infra.connect(config, &env_vars, &environment).await?;
        let client = Arc::new(C::from(client));
        self.insert_clients(server_name, client).await?;

        Ok(())
    }

    async fn init_mcp(&self) -> anyhow::Result<()> {
        let mcp = self.manager.read_mcp_config(None).await?;

        // Fast path: if config is unchanged, skip reinitialization without acquiring
        // the lock
        if !self.is_config_modified(&mcp).await {
            return Ok(());
        }

        // Serialise concurrent initialisations so only one caller runs update_mcp at a
        // time
        let _guard = self.init_lock.lock().await;

        // Double-check under the lock: a concurrent caller may have already updated
        if !self.is_config_modified(&mcp).await {
            return Ok(());
        }

        self.update_mcp(mcp).await
    }

    async fn update_mcp(&self, mcp: McpConfig) -> Result<(), anyhow::Error> {
        // Compute the hash early before mcp is consumed, but write it only after
        // all connections are established so waiters on init_lock see a consistent
        // state.
        let new_hash = mcp.cache_key();
        self.clear_tools().await;

        // Clear failed servers map before attempting new connections
        self.failed_servers.write().await.clear();

        let connections: Vec<_> = mcp
            .mcp_servers
            .into_iter()
            .filter(|v| !v.1.is_disabled())
            .map(|(name, server)| async move {
                let conn = self
                    .connect(&name, server)
                    .await
                    .context(format!("Failed to initiate MCP server: {name}"));

                (name, conn)
            })
            .collect();

        let results = futures::future::join_all(connections).await;

        for (server_name, result) in results {
            match result {
                Ok(_) => {}
                Err(error) => {
                    // Format error with full chain for detailed diagnostics
                    // Using Debug formatting with alternate flag shows the full error chain
                    let error_string = format!("{error:?}");
                    self.failed_servers
                        .write()
                        .await
                        .insert(server_name.clone(), error_string.clone());
                }
            }
        }

        // Write the hash only after join_all finishes so that any waiter on
        // init_lock re-checks is_config_modified only once self.tools is fully
        // populated, preventing "Tool not found" races.
        *self.previous_config_hash.lock().await = new_hash;

        Ok(())
    }

    async fn list(&self) -> anyhow::Result<McpServers> {
        self.init_mcp().await?;

        let tools = self.tools.read().await;
        let mut grouped_tools = std::collections::HashMap::new();

        for tool in tools.values() {
            grouped_tools
                .entry(ServerName::from(tool.server_name.clone()))
                .or_insert_with(Vec::new)
                .push(tool.definition.clone());
        }

        let failures = self.failed_servers.read().await.clone();

        Ok(McpServers::new(grouped_tools, failures))
    }
    async fn clear_tools(&self) {
        self.tools.write().await.clear()
    }

    async fn call(&self, call: ToolCallFull) -> anyhow::Result<ToolOutput> {
        // Ensure MCP connections are initialized before calling tools
        self.init_mcp().await?;

        let tools = self.tools.read().await;

        // Try exact match first, then fall back to legacy-format lookup for
        // tool calls arriving in the Claude Code `mcp__{server}__{tool}` format.
        let tool = tools
            .get(&call.name)
            .or_else(|| call.name.to_legacy_mcp_name().and_then(|n| tools.get(&n)))
            .context("Tool not found")?;

        tool.executable.call_tool(call.arguments.parse()?).await
    }

    /// Refresh the MCP cache by clearing cached data.
    /// Does NOT eagerly connect to servers - connections happen lazily
    /// when list() or call() is invoked, avoiding interactive OAuth during
    /// reload.
    async fn refresh_cache(&self) -> anyhow::Result<()> {
        // Hold init_lock so we don't race with an in-flight update_mcp: without
        // this, clear_tools could run while connections are still being
        // established, leaving waiters released into an empty tool map.
        let _guard = self.init_lock.lock().await;
        // Clear the infra cache and reset config hash to force re-init on next access
        self.infra.cache_clear().await?;
        *self.previous_config_hash.lock().await = Default::default();
        self.clear_tools().await;
        self.failed_servers.write().await.clear();
        Ok(())
    }
}

#[async_trait::async_trait]
impl<M: McpConfigManager, I: McpServerInfra + KVStore + EnvironmentInfra, C> McpService
    for ForgeMcpService<M, I, C>
where
    C: McpClientInfra + Clone,
    C: From<<I as McpServerInfra>::Client>,
{
    async fn get_mcp_servers(&self) -> anyhow::Result<McpServers> {
        // Read current configs to compute merged hash
        let mcp_config = self.manager.read_mcp_config(None).await?;

        // Compute unified hash from merged config
        let config_hash = mcp_config.cache_key();

        // Check if cache is valid (exists and not expired)
        // Cache is valid, retrieve it
        if let Some(cache) = self.infra.cache_get::<_, McpServers>(&config_hash).await? {
            return Ok(cache.clone());
        }

        let servers = self.list().await?;
        self.infra.cache_set(&config_hash, &servers).await?;
        Ok(servers)
    }

    async fn execute_mcp(&self, call: ToolCallFull) -> anyhow::Result<ToolOutput> {
        self.call(call).await
    }

    async fn reload_mcp(&self) -> anyhow::Result<()> {
        self.refresh_cache().await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use fake::{Fake, Faker};
    use forge_app::domain::{
        ConfigOperation, Environment, McpConfig, McpServerConfig, Scope, ServerName, ToolCallFull,
        ToolDefinition, ToolName, ToolOutput,
    };
    use forge_app::{
        EnvironmentInfra, KVStore, McpClientInfra, McpConfigManager, McpServerInfra, McpService,
    };
    use forge_config::ForgeConfig;
    use pretty_assertions::assert_eq;
    use serde::de::DeserializeOwned;

    use super::{ForgeMcpService, generate_mcp_tool_name};

    // ── Mock MCP client ──────────────────────────────────────────────────────

    #[derive(Clone)]
    struct MockMcpClient;

    #[async_trait::async_trait]
    impl McpClientInfra for MockMcpClient {
        async fn list(&self) -> anyhow::Result<Vec<ToolDefinition>> {
            Ok(vec![ToolDefinition::new("test_tool")])
        }

        async fn call(
            &self,
            _tool_name: &ToolName,
            _input: serde_json::Value,
        ) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput::text("mock result"))
        }
    }

    // ── Mock config manager ──────────────────────────────────────────────────

    struct MockMcpManager;

    #[async_trait::async_trait]
    impl McpConfigManager for MockMcpManager {
        async fn read_mcp_config(&self, _scope: Option<&Scope>) -> anyhow::Result<McpConfig> {
            let mut servers = BTreeMap::new();
            servers.insert(
                ServerName::from("test-server".to_string()),
                McpServerConfig::new_stdio("echo", vec![], None),
            );
            Ok(McpConfig { mcp_servers: servers })
        }

        async fn write_mcp_config(
            &self,
            _config: &McpConfig,
            _scope: &Scope,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    // ── Mock infrastructure ──────────────────────────────────────────────────

    #[derive(Clone)]
    struct MockInfra;

    #[async_trait::async_trait]
    impl McpServerInfra for MockInfra {
        type Client = MockMcpClient;

        async fn connect(
            &self,
            _config: McpServerConfig,
            _env_vars: &BTreeMap<String, String>,
            _environment: &Environment,
        ) -> anyhow::Result<MockMcpClient> {
            Ok(MockMcpClient)
        }
    }

    #[async_trait::async_trait]
    impl KVStore for MockInfra {
        async fn cache_get<K, V>(&self, _key: &K) -> anyhow::Result<Option<V>>
        where
            K: std::hash::Hash + Sync,
            V: serde::Serialize + DeserializeOwned + Send,
        {
            Ok(None)
        }

        async fn cache_set<K, V>(&self, _key: &K, _value: &V) -> anyhow::Result<()>
        where
            K: std::hash::Hash + Sync,
            V: serde::Serialize + Sync,
        {
            Ok(())
        }

        async fn cache_clear(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    impl EnvironmentInfra for MockInfra {
        type Config = ForgeConfig;

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }

        fn get_environment(&self) -> Environment {
            Faker.fake()
        }

        fn get_config(&self) -> anyhow::Result<ForgeConfig> {
            Ok(ForgeConfig::default())
        }

        async fn update_environment(&self, _ops: Vec<ConfigOperation>) -> anyhow::Result<()> {
            Ok(())
        }
    }

    // ── Fixture ──────────────────────────────────────────────────────────────

    fn fixture() -> ForgeMcpService<MockMcpManager, MockInfra, MockMcpClient> {
        ForgeMcpService::new(Arc::new(MockMcpManager), Arc::new(MockInfra))
    }

    #[test]
    fn test_generate_mcp_tool_name_uses_legacy_format() {
        let fixture = ServerName::from("hugging-face".to_string());
        let actual = generate_mcp_tool_name(&fixture, &ToolName::new("read-channel"));
        let expected = ToolName::new("mcp_hugging_face_tool_read_channel");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_generate_mcp_tool_name_sanitizes_server_and_tool_names() {
        let fixture = ServerName::from("claude.ai Slack".to_string());
        let actual = generate_mcp_tool_name(&fixture, &ToolName::new("Add comment"));
        let expected = ToolName::new("mcp_claude_ai_slack_tool_add_comment");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_to_legacy_mcp_name_converts_claude_code_format() {
        let actual = ToolName::new("mcp__github__create_issue").to_legacy_mcp_name();
        let expected = Some(ToolName::new("mcp_github_tool_create_issue"));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_to_legacy_mcp_name_converts_multipart_server_name() {
        let actual = ToolName::new("mcp__hugging_face__read_channel").to_legacy_mcp_name();
        let expected = Some(ToolName::new("mcp_hugging_face_tool_read_channel"));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_to_legacy_mcp_name_returns_none_for_non_mcp_tools() {
        let actual = ToolName::new("read").to_legacy_mcp_name();
        assert_eq!(actual, None);
    }

    #[test]
    fn test_to_legacy_mcp_name_returns_none_for_legacy_format() {
        // Already in legacy format — should not double-convert
        let actual = ToolName::new("mcp_github_tool_create_issue").to_legacy_mcp_name();
        assert_eq!(actual, None);
    }

    // ── Concurrent initialisation test ──────────────────────────────────────

    /// Verify that two concurrent callers of `get_mcp_servers` do not race:
    /// after both futures settle, every registered tool must be callable
    /// without a "Tool not found" error.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_init_does_not_race() {
        let service = Arc::new(fixture());

        let s1 = service.clone();
        let s2 = service.clone();
        let (r1, r2) = tokio::join!(s1.get_mcp_servers(), s2.get_mcp_servers());
        r1.unwrap();
        r2.unwrap();

        let servers = service.get_mcp_servers().await.unwrap();
        let tool_name = servers
            .get_servers()
            .values()
            .flat_map(|tools| tools.iter())
            .next()
            .expect("at least one tool must be registered")
            .name
            .clone();

        let call = ToolCallFull::new(tool_name);
        let actual = service.execute_mcp(call).await.unwrap();
        let expected = ToolOutput::text("mock result");
        assert_eq!(actual, expected);
    }
}
