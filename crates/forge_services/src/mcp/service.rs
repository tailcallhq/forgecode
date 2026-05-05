use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

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

const DEFAULT_MCP_SERVER_TIMEOUT_SECS: u64 = 300;

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

    fn server_timeout(config: &McpServerConfig) -> Duration {
        let seconds = match config {
            McpServerConfig::Stdio(stdio) => stdio.timeout,
            McpServerConfig::Http(http) => http.timeout,
        }
        .unwrap_or(DEFAULT_MCP_SERVER_TIMEOUT_SECS);

        Duration::from_secs(seconds)
    }

    async fn build_tool_holders(
        &self,
        server_name: &ServerName,
        client: Arc<C>,
    ) -> anyhow::Result<Vec<(ToolName, ToolHolder<McpExecutor<C>>)>> {
        let tools = client.list().await?;
        let mut tool_holders = Vec::with_capacity(tools.len());

        for mut tool in tools.into_iter() {
            let actual_name = tool.name.clone();
            let server = McpExecutor::new(actual_name, client.clone())?;
            let generated_name = generate_mcp_tool_name(server_name, &tool.name);

            tool.name = generated_name.clone();

            tool_holders.push((
                generated_name,
                ToolHolder {
                    definition: tool,
                    executable: server,
                    server_name: server_name.to_string(),
                },
            ));
        }

        Ok(tool_holders)
    }

    async fn load_server_tools(
        &self,
        server_name: &ServerName,
        config: McpServerConfig,
    ) -> anyhow::Result<Vec<(ToolName, ToolHolder<McpExecutor<C>>)>> {
        let env_vars = self.infra.get_env_vars();
        let environment = self.infra.get_environment();
        let client = self.infra.connect(config, &env_vars, &environment).await?;
        let client = Arc::new(C::from(client));
        self.build_tool_holders(server_name, client).await
    }

    async fn init_mcp(&self) -> anyhow::Result<()> {
        let mcp = self.manager.read_mcp_config(None).await?;

        if !self.is_config_modified(&mcp).await {
            return Ok(());
        }

        let _guard = self.init_lock.lock().await;

        if !self.is_config_modified(&mcp).await {
            return Ok(());
        }

        self.update_mcp(mcp).await
    }

    async fn update_mcp(&self, mcp: McpConfig) -> anyhow::Result<()> {
        let new_hash = mcp.cache_key();

        let connections: Vec<_> = mcp
            .mcp_servers
            .into_iter()
            .filter(|(_, server)| !server.is_disabled())
            .map(|(server_name, server)| {
                let timeout = Self::server_timeout(&server);
                async move {
                    let result = match tokio::time::timeout(
                        timeout,
                        self.load_server_tools(&server_name, server),
                    )
                    .await
                    {
                        Ok(result) => result.with_context(|| {
                            format!("Failed to initiate MCP server: {server_name}")
                        }),
                        Err(error) => Err(anyhow::Error::new(error).context(format!(
                            "Timed out after {}s while loading MCP server: {server_name}",
                            timeout.as_secs()
                        ))),
                    };

                    (server_name, result)
                }
            })
            .collect();

        let results = futures::future::join_all(connections).await;
        let mut tool_map = HashMap::new();
        let mut failed_servers = HashMap::new();

        for (server_name, result) in results {
            match result {
                Ok(holders) => {
                    tool_map.extend(holders.into_iter());
                }
                Err(error) => {
                    failed_servers.insert(server_name, format!("{error:?}"));
                }
            }
        }

        *self.tools.write().await = tool_map;
        *self.failed_servers.write().await = failed_servers;
        *self.previous_config_hash.lock().await = new_hash;

        Ok(())
    }

    async fn list(&self) -> anyhow::Result<McpServers> {
        self.init_mcp().await?;

        let tools = self.tools.read().await;
        let mut grouped_tools = HashMap::new();

        for tool in tools.values() {
            grouped_tools
                .entry(ServerName::from(tool.server_name.clone()))
                .or_insert_with(Vec::new)
                .push(tool.definition.clone());
        }

        let failures = self.failed_servers.read().await.clone();

        Ok(McpServers::new(grouped_tools, failures))
    }

    async fn call(&self, call: ToolCallFull) -> anyhow::Result<ToolOutput> {
        self.init_mcp().await?;

        let tools = self.tools.read().await;
        let tool = tools
            .get(&call.name)
            .or_else(|| {
                call.name
                    .to_legacy_mcp_name()
                    .and_then(|name| tools.get(&name))
            })
            .context("Tool not found")?;

        tool.executable.call_tool(call.arguments.parse()?).await
    }

    /// Refresh the MCP cache by invalidating cached data.
    /// Does NOT eagerly connect to servers - connections happen lazily
    /// when list() or call() is invoked, avoiding interactive OAuth during
    /// reload. The last known tool map stays in memory until the next
    /// initialization completes so callers never observe an empty registry
    /// during a refresh.
    async fn refresh_cache(&self) -> anyhow::Result<()> {
        let _guard = self.init_lock.lock().await;
        self.infra.cache_clear().await?;
        *self.previous_config_hash.lock().await = Default::default();
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
        let mcp_config = self.manager.read_mcp_config(None).await?;
        let config_hash = mcp_config.cache_key();

        if let Some(cache) = self.infra.cache_get::<_, McpServers>(&config_hash).await? {
            return Ok(cache.clone());
        }

        let actual = self.list().await?;
        self.infra.cache_set(&config_hash, &actual).await?;
        Ok(actual)
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
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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

    struct MockMcpManager;

    #[derive(Clone)]
    struct StatefulMcpManager {
        config: Arc<Mutex<McpConfig>>,
    }

    impl StatefulMcpManager {
        fn new(config: McpConfig) -> Self {
            Self { config: Arc::new(Mutex::new(config)) }
        }

        fn set_config(&self, config: McpConfig) {
            *self.config.lock().expect("config mutex poisoned") = config;
        }
    }

    #[async_trait::async_trait]
    impl McpConfigManager for StatefulMcpManager {
        async fn read_mcp_config(&self, _scope: Option<&Scope>) -> anyhow::Result<McpConfig> {
            Ok(self.config.lock().expect("config mutex poisoned").clone())
        }

        async fn write_mcp_config(
            &self,
            _config: &McpConfig,
            _scope: &Scope,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

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

    #[derive(Clone)]
    struct MockInfra;

    #[async_trait::async_trait]
    impl McpServerInfra for MockInfra {
        type Client = MockMcpClient;

        async fn connect(
            &self,
            config: McpServerConfig,
            _env_vars: &BTreeMap<String, String>,
            _environment: &Environment,
        ) -> anyhow::Result<MockMcpClient> {
            if let McpServerConfig::Stdio(stdio) = &config {
                if stdio.command == "slow" {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }

                if stdio.command == "broken" {
                    return Err(anyhow::anyhow!("broken server"));
                }
            }

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

    fn fixture() -> ForgeMcpService<MockMcpManager, MockInfra, MockMcpClient> {
        ForgeMcpService::new(Arc::new(MockMcpManager), Arc::new(MockInfra))
    }

    fn fixture_with_manager<M>(manager: Arc<M>) -> ForgeMcpService<M, MockInfra, MockMcpClient>
    where
        M: McpConfigManager,
    {
        ForgeMcpService::new(manager, Arc::new(MockInfra))
    }

    fn stdio_server(command: &str, timeout: Option<u64>) -> McpServerConfig {
        let mut actual = McpServerConfig::new_stdio(command, vec![], None);
        if let McpServerConfig::Stdio(stdio) = &mut actual {
            stdio.timeout = timeout;
        }
        actual
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
        let expected = None;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_to_legacy_mcp_name_returns_none_for_legacy_format() {
        let actual = ToolName::new("mcp_github_tool_create_issue").to_legacy_mcp_name();
        let expected = None;
        assert_eq!(actual, expected);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_init_does_not_race() {
        let setup = Arc::new(fixture());

        let fixture_one = setup.clone();
        let fixture_two = setup.clone();
        let (actual_one, actual_two) =
            tokio::join!(fixture_one.get_mcp_servers(), fixture_two.get_mcp_servers());
        actual_one.unwrap();
        actual_two.unwrap();

        let actual = setup.get_mcp_servers().await.unwrap();
        let tool_name = actual
            .get_servers()
            .values()
            .flat_map(|tools| tools.iter())
            .next()
            .expect("at least one tool must be registered")
            .name
            .clone();

        let actual = setup
            .execute_mcp(ToolCallFull::new(tool_name))
            .await
            .unwrap();
        let expected = ToolOutput::text("mock result");
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_reload_mcp_preserves_last_known_tools_until_next_init() {
        let setup = McpConfig {
            mcp_servers: BTreeMap::from([(
                ServerName::from("initial-server".to_string()),
                stdio_server("fast", Some(1)),
            )]),
        };
        let fixture = Arc::new(StatefulMcpManager::new(setup));
        let service = fixture_with_manager(fixture.clone());

        let actual = service.get_mcp_servers().await.unwrap();
        let expected = 1;
        assert_eq!(actual.get_servers().len(), expected);

        fixture.set_config(McpConfig {
            mcp_servers: BTreeMap::from([(
                ServerName::from("updated-server".to_string()),
                stdio_server("fast", Some(1)),
            )]),
        });

        service.reload_mcp().await.unwrap();

        let actual = service.tools.read().await.len();
        let expected = 1;
        assert_eq!(actual, expected);

        let actual = service.get_mcp_servers().await.unwrap();
        let expected = true;
        assert_eq!(
            actual
                .get_servers()
                .contains_key(&ServerName::from("updated-server".to_string())),
            expected
        );
    }

    #[tokio::test]
    async fn test_get_mcp_servers_keeps_successful_servers_when_one_server_times_out() {
        let fixture = Arc::new(StatefulMcpManager::new(McpConfig {
            mcp_servers: BTreeMap::from([
                (
                    ServerName::from("fast-server".to_string()),
                    stdio_server("fast", Some(1)),
                ),
                (
                    ServerName::from("slow-server".to_string()),
                    stdio_server("slow", Some(0)),
                ),
            ]),
        }));
        let service = fixture_with_manager(fixture);

        let actual = service.get_mcp_servers().await.unwrap();
        let expected = 1;
        assert_eq!(actual.get_servers().len(), expected);

        let actual_fast = actual
            .get_servers()
            .contains_key(&ServerName::from("fast-server".to_string()));
        let expected_fast = true;
        assert_eq!(actual_fast, expected_fast);

        let actual_failure = actual
            .get_failures()
            .contains_key(&ServerName::from("slow-server".to_string()));
        let expected_failure = true;
        assert_eq!(actual_failure, expected_failure);

        let tool_name = actual
            .get_servers()
            .get(&ServerName::from("fast-server".to_string()))
            .expect("fast server should be present")
            .first()
            .expect("fast server should expose a tool")
            .name
            .clone();
        let actual = service
            .execute_mcp(ToolCallFull::new(tool_name))
            .await
            .unwrap();
        let expected = ToolOutput::text("mock result");
        assert_eq!(actual, expected);
    }
}
