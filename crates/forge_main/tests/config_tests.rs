//! Integration tests for config loading in the forge_main crate.
//!
//! These tests exercise `forge_config`'s `ConfigReader` (the builder that
//! replaces `config::ConfigBuilder`), `ForgeConfig` default values,
//! TOML serialization roundtrips, environment-variable overrides,
//! and path resolution logic.

use std::sync::{Mutex, MutexGuard};

use forge_config::{
    ConfigReader, ForgeConfig, ModelConfig, ProviderAuthMethod, ProviderResponseType,
    ProviderTypeEntry,
};

// ---------------------------------------------------------------------------
// Helpers – serialise tests that mutate env vars to prevent races.
// ---------------------------------------------------------------------------

static ENV_MUTEX: Mutex<()> = Mutex::new(());

struct EnvGuard {
    keys: Vec<&'static str>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn set(pairs: &[(&'static str, &str)]) -> Self {
        let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let keys = pairs.iter().map(|(k, _)| *k).collect();
        for (key, value) in pairs {
            // SAFETY: tests are single-threaded within the Mutex, and no
            // other thread can observe intermediate state.
            unsafe { std::env::set_var(key, value) };
        }
        Self { keys, _lock: lock }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            unsafe { std::env::remove_var(key) };
        }
    }
}

// ---------------------------------------------------------------------------
// 1. ConfigReader default values
// ---------------------------------------------------------------------------

#[test]
fn reader_defaults_produce_valid_config() {
    let config = ConfigReader::default()
        .read_defaults()
        .build()
        .expect("ConfigReader::default() + read_defaults() + build() must succeed");

    // Verify well-known defaults from the embedded `.forge.toml`.
    assert_eq!(config.max_parallel_file_reads, 64);
    assert_eq!(config.max_read_lines, 2000);
    assert_eq!(config.tool_timeout_secs, 300);
    assert_eq!(config.max_search_lines, 1000);
    assert!(config.tool_supported);
}

#[test]
fn forge_config_default_has_zero_limit_fields() {
    // Fields that default to 0 when no `.forge.toml` is loaded.
    let config = ForgeConfig::default();
    assert_eq!(config.max_search_lines, 0);
    assert_eq!(config.max_search_result_bytes, 0);
    assert_eq!(config.max_fetch_chars, 0);
    assert_eq!(config.max_stdout_prefix_lines, 0);
    assert_eq!(config.max_stdout_suffix_lines, 0);
    assert_eq!(config.max_stdout_line_chars, 0);
    assert_eq!(config.max_line_chars, 0);
    assert_eq!(config.max_read_lines, 0);
    assert_eq!(config.max_file_read_batch_size, 0);
    assert_eq!(config.max_file_size_bytes, 0);
    assert_eq!(config.tool_timeout_secs, 0);
    assert_eq!(config.max_conversations, 0);
    assert_eq!(config.max_parallel_file_reads, 0);
}

#[test]
fn forge_config_default_optional_fields_are_none() {
    let config = ForgeConfig::default();
    assert!(config.session.is_none());
    assert!(config.commit.is_none());
    assert!(config.suggest.is_none());
    assert!(config.http.is_none());
    assert!(config.auto_dump.is_none());
    assert!(config.temperature.is_none());
    assert!(config.top_p.is_none());
    assert!(config.top_k.is_none());
    assert!(config.max_tokens.is_none());
    assert!(config.compact.is_none());
    assert!(config.output.is_none());
    assert!(config.reasoning.is_none());
    assert!(config.updates.is_none());
}

#[test]
fn forge_config_default_bool_flags() {
    let config = ForgeConfig::default();
    assert!(!config.auto_open_dump);
    assert!(!config.auto_continue_on_interrupt);
    assert!(!config.restricted);
    assert!(!config.verify_todos);
    assert!(!config.use_text_patch_fallback);
    assert!(!config.research_subagent);
    assert!(!config.subagents);
    assert!(!config.merge_system_messages);
}

#[test]
fn forge_config_default_string_fields() {
    // Raw Rust Default: empty strings.
    let raw = ForgeConfig::default();
    assert_eq!(raw.services_url, "");
    assert_eq!(raw.currency_symbol, "");

    // Loaded defaults from .forge.toml: currency_symbol gets "$".
    let loaded = ConfigReader::default()
        .read_defaults()
        .build()
        .expect("read_defaults must succeed");
    assert_eq!(loaded.currency_symbol, "$");
}

#[test]
fn forge_config_default_providers_empty() {
    let config = ForgeConfig::default();
    assert!(config.providers.is_empty());
}

// ---------------------------------------------------------------------------
// 2. ConfigReader with_overrides / read_toml works
// ---------------------------------------------------------------------------

#[test]
fn read_toml_overrides_defaults() {
    let toml = r#"
        max_search_lines = 999
        tool_timeout_secs = 42
        restricted = true
        currency_symbol = "EUR"
    "#;

    let config = ConfigReader::default()
        .read_defaults()
        .read_toml(toml)
        .build()
        .expect("build must succeed");

    assert_eq!(config.max_search_lines, 999);
    assert_eq!(config.tool_timeout_secs, 42);
    assert!(config.restricted);
    assert_eq!(config.currency_symbol, "EUR");
}

#[test]
fn read_toml_session_model_overrides() {
    let toml = r#"
[session]
provider_id = "custom_provider"
model_id = "custom-model-1"
    "#;

    let config = ConfigReader::default()
        .read_defaults()
        .read_toml(toml)
        .build()
        .expect("build must succeed");

    let session = config.session.expect("session must be Some after override");
    assert_eq!(session.provider_id, "custom_provider");
    assert_eq!(session.model_id, "custom-model-1");
}

#[test]
fn read_toml_commit_model_overrides() {
    let toml = r#"
[commit]
provider_id = "anthropic"
model_id = "claude-3-5-sonnet"
    "#;

    let config = ConfigReader::default()
        .read_defaults()
        .read_toml(toml)
        .build()
        .expect("build must succeed");

    let commit = config.commit.expect("commit must be Some after override");
    assert_eq!(commit.provider_id, "anthropic");
    assert_eq!(commit.model_id, "claude-3-5-sonnet");
}

#[test]
fn read_toml_provider_entry_roundtrip() {
    let toml = r#"
[[providers]]
id = "my_custom_provider"
url = "http://localhost:8080/v1/chat/completions"
response_type = "OpenAI"
api_key_var = "MY_CUSTOM_API_KEY"
auth_methods = ["api_key"]

[[providers.models]]
id = "custom-model-v1"
name = "Custom Model V1"
description = "A test model"
context_length = 131072
tools_supported = true
supports_reasoning = false
input_modalities = ["text"]
    "#;

    let config = ConfigReader::default()
        .read_toml(toml)
        .build()
        .expect("build must succeed");

    assert_eq!(config.providers.len(), 1);
    let entry = &config.providers[0];
    assert_eq!(entry.id, "my_custom_provider");
    assert_eq!(entry.url, "http://localhost:8080/v1/chat/completions");
    assert_eq!(entry.response_type, Some(ProviderResponseType::OpenAI));
    assert_eq!(entry.api_key_var.as_deref(), Some("MY_CUSTOM_API_KEY"));
    assert_eq!(entry.auth_methods, vec![ProviderAuthMethod::ApiKey]);
    assert!(entry.models.is_some());
}

#[test]
fn read_toml_provider_with_google_adc_auth() {
    let toml = r#"
[[providers]]
id = "vertex_test"
url = "https://us-central1-aiplatform.googleapis.com/v1/chat/completions"
auth_methods = ["google_adc"]
    "#;

    let config = ConfigReader::default()
        .read_toml(toml)
        .build()
        .expect("build must succeed");

    let entry = &config.providers[0];
    assert_eq!(entry.auth_methods, vec![ProviderAuthMethod::GoogleAdc]);
}

#[test]
fn read_toml_provider_type_context_engine() {
    let toml = r#"
[[providers]]
id = "context_index"
url = "http://localhost:9000/search"
provider_type = "context_engine"
    "#;

    let config = ConfigReader::default()
        .read_toml(toml)
        .build()
        .expect("build must succeed");

    let entry = &config.providers[0];
    assert_eq!(entry.provider_type, Some(ProviderTypeEntry::ContextEngine));
}

#[test]
fn read_toml_compact_config() {
    let toml = r#"
[compact]
retention_window = 10
enable_prefilter = true
summary_timeout_secs = 5
    "#;

    let config = ConfigReader::default()
        .read_defaults()
        .read_toml(toml)
        .build()
        .expect("build must succeed");

    let compact = config.compact.expect("compact must be Some");
    assert_eq!(compact.retention_window, 10);
    assert!(compact.enable_prefilter);
    assert_eq!(compact.summary_timeout_secs, 5);
}

#[test]
fn read_toml_reasoning_config() {
    let toml = r#"
[reasoning]
effort = "high"
max_tokens = 8192
enabled = true
    "#;

    let config = ConfigReader::default()
        .read_toml(toml)
        .build()
        .expect("build must succeed");

    let reasoning = config.reasoning.expect("reasoning must be Some");
    assert_eq!(reasoning.effort, Some(forge_config::Effort::High));
    assert_eq!(reasoning.max_tokens, Some(8192));
    assert_eq!(reasoning.enabled, Some(true));
}

// ---------------------------------------------------------------------------
// 3. Environment variable detection
// ---------------------------------------------------------------------------

#[test]
fn env_var_overrides_session() {
    let _guard = EnvGuard::set(&[
        ("FORGE_SESSION__PROVIDER_ID", "env_provider"),
        ("FORGE_SESSION__MODEL_ID", "env_model"),
    ]);

    let config = ConfigReader::default()
        .read_defaults()
        .read_env()
        .build()
        .expect("build must succeed");

    let session = config.session.expect("session must be set from env");
    assert_eq!(session.provider_id, "env_provider");
    assert_eq!(session.model_id, "env_model");
}

#[test]
fn env_var_overrides_max_search_lines() {
    let _guard = EnvGuard::set(&[("FORGE_MAX_SEARCH_LINES", "4242")]);

    let config = ConfigReader::default()
        .read_defaults()
        .read_env()
        .build()
        .expect("build must succeed");

    assert_eq!(config.max_search_lines, 4242);
}

#[test]
fn env_var_overrides_restricted_flag() {
    let _guard = EnvGuard::set(&[("FORGE_RESTRICTED", "true")]);

    let config = ConfigReader::default()
        .read_defaults()
        .read_env()
        .build()
        .expect("build must succeed");

    assert!(config.restricted);
}

#[test]
fn env_var_overrides_tool_timeout() {
    let _guard = EnvGuard::set(&[("FORGE_TOOL_TIMEOUT_SECS", "600")]);

    let config = ConfigReader::default()
        .read_defaults()
        .read_env()
        .build()
        .expect("build must succeed");

    assert_eq!(config.tool_timeout_secs, 600);
}

#[test]
fn toml_takes_precedence_over_defaults_but_env_beats_both() {
    // Layer: defaults < toml < env
    let _guard = EnvGuard::set(&[("FORGE_MAX_SEARCH_LINES", "7777")]);

    let toml = "max_search_lines = 5555\n";

    let config = ConfigReader::default()
        .read_defaults()
        .read_toml(toml)
        .read_env()
        .build()
        .expect("build must succeed");

    // Environment wins over toml.
    assert_eq!(config.max_search_lines, 7777);
}

// ---------------------------------------------------------------------------
// 4. Config file path resolution
// ---------------------------------------------------------------------------

#[test]
fn config_legacy_path_ends_with_config_json() {
    let path = ConfigReader::config_legacy_path();
    let file_name = path.file_name().unwrap().to_string_lossy();
    assert_eq!(file_name.as_ref(), ".config.json");
}

#[test]
fn config_path_ends_with_toml() {
    let path = ConfigReader::config_path();
    let ext = path.extension().unwrap().to_string_lossy();
    assert_eq!(ext.as_ref(), "toml");
}

#[test]
fn cache_path_is_under_base() {
    let cache = ConfigReader::cache_path();
    let base = ConfigReader::base_path();
    assert!(cache.starts_with(&base));
    assert_eq!(
        cache.file_name().unwrap().to_string_lossy().as_ref(),
        "cache"
    );
}

#[test]
fn logs_path_is_under_base() {
    let logs = ConfigReader::logs_path();
    let base = ConfigReader::base_path();
    assert!(logs.starts_with(&base));
    assert_eq!(logs.file_name().unwrap().to_string_lossy().as_ref(), "logs");
}

#[test]
fn locks_path_is_under_base() {
    let locks = ConfigReader::locks_path();
    let base = ConfigReader::base_path();
    assert!(locks.starts_with(&base));
    assert_eq!(
        locks.file_name().unwrap().to_string_lossy().as_ref(),
        "locks"
    );
}

#[test]
fn sessions_path_is_under_base() {
    let sessions = ConfigReader::sessions_path();
    let base = ConfigReader::base_path();
    assert!(sessions.starts_with(&base));
    assert_eq!(
        sessions.file_name().unwrap().to_string_lossy().as_ref(),
        "sessions"
    );
}

// ---------------------------------------------------------------------------
// 5. ModelConfig construction and equality
// ---------------------------------------------------------------------------

#[test]
fn model_config_new_and_equality() {
    let a = ModelConfig::new("anthropic", "claude-3-opus");
    let b = ModelConfig {
        provider_id: "anthropic".to_string(),
        model_id: "claude-3-opus".to_string(),
    };
    assert_eq!(a, b);
}

#[test]
fn model_config_serialization_roundtrip() {
    let original = ModelConfig::new("openai", "gpt-4o");
    let json = serde_json::to_string(&original).unwrap();
    let deserialized: ModelConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(original, deserialized);
}

// ---------------------------------------------------------------------------
// 6. ForgeConfig TOML roundtrip
// ---------------------------------------------------------------------------

#[test]
fn forge_config_toml_roundtrip_preserves_all_fields() {
    let original = ForgeConfig {
        max_search_lines: 42,
        tool_timeout_secs: 120,
        restricted: true,
        currency_symbol: "EUR".to_string(),
        session: Some(ModelConfig::new("anthropic", "claude-3-opus")),
        ..Default::default()
    };

    let toml = toml_edit::ser::to_string_pretty(&original).unwrap();
    let roundtripped = ConfigReader::default()
        .read_defaults()
        .read_toml(&toml)
        .build()
        .expect("roundtrip build must succeed");

    assert_eq!(roundtripped.max_search_lines, 42);
    assert_eq!(roundtripped.tool_timeout_secs, 120);
    assert!(roundtripped.restricted);
    assert_eq!(roundtripped.currency_symbol, "EUR");
    assert_eq!(
        roundtripped.session,
        Some(ModelConfig::new("anthropic", "claude-3-opus"))
    );
}

#[test]
fn forge_config_json_roundtrip_preserves_session() {
    let original = ForgeConfig {
        session: Some(ModelConfig::new("openai", "gpt-4o-mini")),
        commit: Some(ModelConfig::new("anthropic", "claude-3-5-haiku")),
        ..Default::default()
    };

    let json = serde_json::to_string(&original).unwrap();
    let deserialized: ForgeConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(original.session, deserialized.session);
    assert_eq!(original.commit, deserialized.commit);
}
