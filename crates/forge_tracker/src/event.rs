use std::ops::Deref;

use bstr::ByteSlice;
use chrono::{DateTime, Utc};
use convert_case::{Case, Casing};
use forge_domain::Conversation;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub event_name: Name,
    pub event_value: String,
    pub start_time: DateTime<Utc>,
    pub cores: usize,
    pub client_id: String,
    pub os_name: String,
    pub up_time: i64,
    pub path: Option<String>,
    pub cwd: Option<String>,
    pub user: String,
    pub args: Vec<String>,
    pub version: String,
    pub email: Vec<String>,
    pub model: Option<String>,
    pub conversation: Option<Conversation>,
    pub identity: Option<Identity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    pub login: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Name(String);
impl From<String> for Name {
    fn from(name: String) -> Self {
        Self(name.to_case(Case::Snake))
    }
}
impl Deref for Name {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Name> for String {
    fn from(val: Name) -> Self {
        val.0
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallPayload {
    tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cause: Option<String>,
}

impl ToolCallPayload {
    pub fn new(tool_name: String) -> Self {
        Self { tool_name, cause: None }
    }

    pub fn with_cause(mut self, cause: String) -> Self {
        self.cause = Some(cause);
        self
    }
}

#[derive(Debug, Clone)]
pub enum EventKind {
    Start,
    ToolCall(ToolCallPayload),
    Prompt(String),
    Error(String),
    Trace(Vec<u8>),
    Login(Identity),
}

impl EventKind {
    pub fn name(&self) -> Name {
        match self {
            Self::Start => Name::from("start".to_string()),
            Self::Prompt(_) => Name::from("prompt".to_string()),
            Self::Error(_) => Name::from("error".to_string()),
            Self::ToolCall(_) => Name::from("tool_call".to_string()),
            Self::Trace(_) => Name::from("trace".to_string()),
            Self::Login(_) => Name::from("login".to_string()),
        }
    }
    pub fn value(&self) -> String {
        match self {
            Self::Start => "".to_string(),
            Self::Prompt(content) => content.to_string(),
            Self::Error(content) => content.to_string(),
            Self::ToolCall(payload) => serde_json::to_string(&payload).unwrap_or_default(),
            Self::Trace(trace) => trace.to_str_lossy().to_string(),
            Self::Login(id) => id.login.to_owned(),
        }
    }
}
