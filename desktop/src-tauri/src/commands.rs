use std::path::PathBuf;

use anyhow::Result;
use forge_api::{API as _, ChatRequest, ForgeAPI};
use forge_config::ForgeConfig;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Shared application state managed by Tauri
// ---------------------------------------------------------------------------

/// Concrete service/infra types used by the desktop app.
type Infra = forge_infra::ForgeInfra;
type Repo = forge_repo::ForgeRepo<Infra>;
type Services = forge_services::ForgeServices<Repo>;
type Api = ForgeAPI<Services, Repo>;

/// Wraps the forge API instance behind a `RwLock` so Tauri commands can
/// access it concurrently.
pub struct AppState {
    api: RwLock<Option<Api>>,
}

impl AppState {
    pub fn new() -> Self {
        Self { api: RwLock::new(None) }
    }
}

/// Ensure the forge API is initialised on first use.
async fn ensure_api(state: &tauri::State<'_, AppState>) -> Result<()> {
    let mut guard = state.api.write().await;
    if guard.is_none() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config = ForgeConfig::read().unwrap_or_default();
        *guard = Some(ForgeAPI::init(cwd, config));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Serializable DTOs returned to the frontend
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct GreetPayload {
    pub name: String,
}

#[derive(Serialize, Deserialize)]
pub struct RunAgentPayload {
    pub prompt: String,
    /// Optional working directory override.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ChatPayload {
    pub message: String,
}

#[derive(Serialize, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub context_length: u64,
    pub tools_supported: bool,
}

#[derive(Serialize, Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Serialize, Clone)]
pub struct ConfigInfo {
    pub session: Option<String>,
    pub provider: Option<String>,
    pub cwd: String,
    pub version: String,
}

#[derive(Serialize)]
pub struct AgentRunResult {
    pub success: bool,
    pub message: String,
}

#[derive(Serialize)]
pub struct ChatResult {
    pub response: String,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Simple greet command — verifies the Tauri invoke bridge works.
#[tauri::command]
pub fn greet(payload: GreetPayload) -> String {
    format!("Hello, {}! Welcome to ForgeCode Desktop.", payload.name)
}

/// Return the list of available LLM models.
#[tauri::command]
pub async fn get_models(state: tauri::State<'_, AppState>) -> Result<Vec<ModelInfo>, String> {
    ensure_api(&state).await.map_err(|e| e.to_string())?;

    let guard = state.api.read().await;
    let api = guard.as_ref().ok_or("API not initialised")?;

    let models = api.get_models().await.map_err(|e| e.to_string())?;

    Ok(models
        .into_iter()
        .map(|m| ModelInfo {
            id: m.id.to_string(),
            name: m.name.unwrap_or_default(),
            provider: m.provider_id.map(|p| p.to_string()).unwrap_or_default(),
            context_length: m.context_length.unwrap_or(0),
            tools_supported: m.tools_supported.unwrap_or(true),
        })
        .collect())
}

/// Return the current configuration state.
#[tauri::command]
pub async fn get_config(state: tauri::State<'_, AppState>) -> Result<ConfigInfo, String> {
    ensure_api(&state).await.map_err(|e| e.to_string())?;

    let guard = state.api.read().await;
    let api = guard.as_ref().ok_or("API not initialised")?;

    let env = api.environment();
    let cwd = env.cwd.display().to_string();

    let session = api.get_session_config().await;
    let version = forge_config::VERSION.to_string();

    Ok(ConfigInfo {
        session: session.as_ref().map(|s| format!("{}/{}", s.provider_id, s.model_id)),
        provider: session.as_ref().map(|s| s.provider_id.clone()),
        cwd,
        version,
    })
}

/// Return the list of available agents.
#[tauri::command]
pub async fn get_agents(state: tauri::State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    ensure_api(&state).await.map_err(|e| e.to_string())?;

    let guard = state.api.read().await;
    let api = guard.as_ref().ok_or("API not initialised")?;

    let agents = api.get_agent_infos().await.map_err(|e| e.to_string())?;

    Ok(agents
        .into_iter()
        .map(|a| AgentInfo {
            id: a.id.to_string(),
            name: a.name,
            description: a.description.unwrap_or_default(),
        })
        .collect())
}

/// Run the agent with the given prompt and return the aggregated response.
///
/// This is a blocking wrapper: it dispatches through the orchestrator and
/// collects the full response stream before returning.
#[tauri::command]
pub async fn run_agent(
    state: tauri::State<'_, AppState>,
    payload: RunAgentPayload,
) -> Result<AgentRunResult, String> {
    ensure_api(&state).await.map_err(|e| e.to_string())?;

    // Optionally change the working directory.
    if let Some(cwd_str) = &payload.cwd {
        let _ = std::env::set_current_dir(cwd_str);
    }

    let guard = state.api.read().await;
    let api = guard.as_ref().ok_or("API not initialised")?;

    let chat_req = ChatRequest::new(payload.prompt);
    let stream = api.chat(chat_req).await.map_err(|e| e.to_string())?;

    // Collect the response stream into a single string.
    use tokio_stream::StreamExt;
    let mut stream = stream;
    let mut response = String::new();

    while let Some(item) = stream.next().await {
        match item {
            Ok(chat_resp) => {
                if let Some(text) = chat_resp.text {
                    response.push_str(&text);
                }
            }
            Err(e) => {
                response.push_str(&format!("\n[error: {e}]"));
            }
        }
    }

    Ok(AgentRunResult { success: true, message: response })
}

/// Send a single chat message and return the streamed response.
#[tauri::command]
pub async fn chat(
    state: tauri::State<'_, AppState>,
    payload: ChatPayload,
) -> Result<ChatResult, String> {
    ensure_api(&state).await.map_err(|e| e.to_string())?;

    let guard = state.api.read().await;
    let api = guard.as_ref().ok_or("API not initialised")?;

    let chat_req = ChatRequest::new(payload.message);
    let stream = api.chat(chat_req).await.map_err(|e| e.to_string())?;

    use tokio_stream::StreamExt;
    let mut stream = stream;
    let mut response = String::new();

    while let Some(item) = stream.next().await {
        match item {
            Ok(chat_resp) => {
                if let Some(text) = chat_resp.text {
                    response.push_str(&text);
                }
            }
            Err(e) => {
                response.push_str(&format!("\n[error: {e}]"));
            }
        }
    }

    Ok(ChatResult { response })
}
