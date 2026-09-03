//! Integration tests for provider resolution in the forge_main crate.
//!
//! These tests exercise `forge_domain`'s `ProviderId` constants,
//! display names, serialization roundtrips, and built-in provider
//! inventory from the forge_main crate's perspective.

use forge_domain::{Model, ProviderId, ProviderResponse, ProviderType};
use std::str::FromStr;

// ---------------------------------------------------------------------------
// 1. ProviderId constant identity
// ---------------------------------------------------------------------------

#[test]
fn all_ten_core_provider_ids_are_distinct() {
    let ids = [
        ProviderId::FORGE,
        ProviderId::OPENAI,
        ProviderId::OPEN_ROUTER,
        ProviderId::ANTHROPIC,
        ProviderId::XAI,
        ProviderId::ZAI,
        ProviderId::VERTEX_AI,
        ProviderId::AZURE,
        ProviderId::BEDROCK,
        ProviderId::GOOGLE_AI_STUDIO,
    ];

    // Every pair must be distinct.
    for (i, a) in ids.iter().enumerate() {
        for (j, b) in ids.iter().enumerate() {
            if i == j {
                continue;
            }
            assert_ne!(
                a, b,
                "ProviderId::{:?} == ProviderId::{:?} (indices {i}, {j})",
                a, b
            );
        }
    }
}

#[test]
fn all_built_in_provider_ids_are_non_empty_strings() {
    for id in ProviderId::built_in_providers() {
        assert!(
            !id.to_string().is_empty(),
            "built-in ProviderId must be non-empty"
        );
    }
}

#[test]
fn built_in_providers_count_at_least_40() {
    let count = ProviderId::built_in_providers().len();
    assert!(
        count >= 40,
        "expected at least 40 built-in providers, got {count}"
    );
}

// ---------------------------------------------------------------------------
// 2. Provider display names
// ---------------------------------------------------------------------------

#[test]
fn display_names_for_core_providers() {
    assert_eq!(ProviderId::FORGE.to_string(), "Forge");
    assert_eq!(ProviderId::OPENAI.to_string(), "OpenAI");
    assert_eq!(ProviderId::OPEN_ROUTER.to_string(), "OpenRouter");
    assert_eq!(ProviderId::ANTHROPIC.to_string(), "Anthropic");
    assert_eq!(ProviderId::XAI.to_string(), "XAI");
    assert_eq!(ProviderId::ZAI.to_string(), "ZAI");
    assert_eq!(ProviderId::ZAI_CODING.to_string(), "ZaiCoding");
    assert_eq!(ProviderId::CEREBRAS.to_string(), "Cerebras");
    assert_eq!(ProviderId::VERTEX_AI.to_string(), "VertexAI");
    assert_eq!(
        ProviderId::VERTEX_AI_ANTHROPIC.to_string(),
        "VertexAIAnthropic"
    );
    assert_eq!(ProviderId::AZURE.to_string(), "Azure");
    assert_eq!(ProviderId::GITHUB_COPILOT.to_string(), "GithubCopilot");
    assert_eq!(ProviderId::BEDROCK.to_string(), "Bedrock");
    assert_eq!(ProviderId::NVIDIA.to_string(), "NVIDIA");
    assert_eq!(ProviderId::META.to_string(), "Meta");
    assert_eq!(ProviderId::CODEX.to_string(), "Codex");
}

#[test]
fn display_names_for_compatible_providers() {
    assert_eq!(
        ProviderId::OPENAI_COMPATIBLE.to_string(),
        "OpenAICompatible"
    );
    assert_eq!(
        ProviderId::OPENAI_RESPONSES_COMPATIBLE.to_string(),
        "OpenAIResponsesCompatible"
    );
    assert_eq!(
        ProviderId::ANTHROPIC_COMPATIBLE.to_string(),
        "AnthropicCompatible"
    );
}

#[test]
fn display_names_for_newer_providers() {
    assert_eq!(ProviderId::IO_INTELLIGENCE.to_string(), "IOIntelligence");
    assert_eq!(ProviderId::MINIMAX.to_string(), "MiniMax");
    assert_eq!(ProviderId::OPENCODE_ZEN.to_string(), "OpenCode Zen");
    assert_eq!(ProviderId::OPENCODE_GO.to_string(), "OpenCode Go");
    assert_eq!(ProviderId::FIREWORKS_AI.to_string(), "FireworksAI");
    assert_eq!(
        ProviderId::FIREWORKS_AI_FIREPASS.to_string(),
        "FireworksAIFirepass"
    );
    assert_eq!(ProviderId::NOVITA.to_string(), "Novita");
    assert_eq!(ProviderId::VIVGRID.to_string(), "Vivgrid");
    assert_eq!(ProviderId::GOOGLE_AI_STUDIO.to_string(), "GoogleAIStudio");
    assert_eq!(ProviderId::MODAL.to_string(), "Modal");
    assert_eq!(ProviderId::ADAL.to_string(), "AdaL");
    assert_eq!(ProviderId::XIAOMI_MIMO.to_string(), "XiaomiMimo");
    assert_eq!(ProviderId::AMBIENT.to_string(), "Ambient");
    assert_eq!(ProviderId::NEURALWATT.to_string(), "Neuralwatt");
    assert_eq!(ProviderId::ORCA_ROUTER.to_string(), "OrcaRouter");
}

// ---------------------------------------------------------------------------
// 3. ProviderId serialization roundtrip (JSON)
// ---------------------------------------------------------------------------

#[test]
fn provider_id_json_roundtrip_for_core() {
    let ids = [
        ProviderId::FORGE,
        ProviderId::OPENAI,
        ProviderId::ANTHROPIC,
        ProviderId::XAI,
        ProviderId::ZAI,
        ProviderId::VERTEX_AI,
        ProviderId::AZURE,
        ProviderId::BEDROCK,
        ProviderId::GOOGLE_AI_STUDIO,
        ProviderId::OPEN_ROUTER,
    ];

    for id in &ids {
        let json = serde_json::to_string(id).unwrap();
        let deserialized: ProviderId = serde_json::from_str(&json).unwrap();
        assert_eq!(*id, deserialized, "roundtrip failed for {id}");
    }
}

#[test]
fn provider_id_json_roundtrip_for_all_built_ins() {
    for id in ProviderId::built_in_providers() {
        let json = serde_json::to_string(id).unwrap();
        let deserialized: ProviderId = serde_json::from_str(&json).unwrap();
        assert_eq!(*id, deserialized, "roundtrip failed for {id}");
    }
}

#[test]
fn provider_id_json_is_quoted_string() {
    let json = serde_json::to_string(&ProviderId::OPENAI).unwrap();
    assert!(
        json.starts_with('"'),
        "JSON must be a quoted string: {json}"
    );
    assert!(json.ends_with('"'), "JSON must be a quoted string: {json}");
    // Value should be "openai".
    assert_eq!(json, "\"openai\"");
}

// ---------------------------------------------------------------------------
// 4. ProviderId FromStr roundtrip
// ---------------------------------------------------------------------------

#[test]
fn from_str_roundtrip_for_core_providers() {
    let cases = [
        ("forge", ProviderId::FORGE),
        ("openai", ProviderId::OPENAI),
        ("open_router", ProviderId::OPEN_ROUTER),
        ("anthropic", ProviderId::ANTHROPIC),
        ("xai", ProviderId::XAI),
        ("zai", ProviderId::ZAI),
        ("vertex_ai", ProviderId::VERTEX_AI),
        ("azure", ProviderId::AZURE),
        ("bedrock", ProviderId::BEDROCK),
        ("google_ai_studio", ProviderId::GOOGLE_AI_STUDIO),
        ("cerebras", ProviderId::CEREBRAS),
        ("nvidia", ProviderId::NVIDIA),
        ("meta", ProviderId::META),
    ];

    for (input, expected) in cases {
        let actual = ProviderId::from_str(input).unwrap();
        assert_eq!(actual, expected, "from_str({input:?}) failed");
    }
}

#[test]
fn custom_provider_id_from_str() {
    let custom = ProviderId::from_str("my_custom_local_llm").unwrap();
    // Custom providers are not built-in, but should round-trip through display.
    assert_eq!(custom.to_string(), "MyCustomLocalLlm");
}

#[test]
fn from_str_is_case_sensitive() {
    // "OpenAI" (capital O) is not a built-in; it becomes a custom provider.
    let id = ProviderId::from_str("OpenAI").unwrap();
    assert_ne!(id, ProviderId::OPENAI);
    assert_eq!(id.to_string(), "OpenAi");
}

// ---------------------------------------------------------------------------
// 5. Built-in providers contain expected members
// ---------------------------------------------------------------------------

#[test]
fn built_in_providers_contains_all_core() {
    let built_in = ProviderId::built_in_providers();
    let expected = [
        ProviderId::FORGE,
        ProviderId::OPENAI,
        ProviderId::OPEN_ROUTER,
        ProviderId::ANTHROPIC,
        ProviderId::XAI,
        ProviderId::ZAI,
        ProviderId::ZAI_CODING,
        ProviderId::CEREBRAS,
        ProviderId::VERTEX_AI,
        ProviderId::VERTEX_AI_ANTHROPIC,
        ProviderId::AZURE,
        ProviderId::GITHUB_COPILOT,
        ProviderId::BEDROCK,
        ProviderId::OPENAI_COMPATIBLE,
        ProviderId::OPENAI_RESPONSES_COMPATIBLE,
        ProviderId::ANTHROPIC_COMPATIBLE,
        ProviderId::FORGE_SERVICES,
        ProviderId::IO_INTELLIGENCE,
        ProviderId::MINIMAX,
        ProviderId::CODEX,
        ProviderId::GOOGLE_AI_STUDIO,
        ProviderId::NVIDIA,
        ProviderId::META,
    ];

    for id in &expected {
        assert!(built_in.contains(id), "built-in list must contain {id}");
    }
}

#[test]
fn built_in_providers_first_is_forge() {
    let built_in = ProviderId::built_in_providers();
    assert_eq!(built_in.first(), Some(&ProviderId::FORGE));
}

#[test]
fn built_in_providers_ordering_is_stable() {
    let built_in = ProviderId::built_in_providers();
    // Verify a known prefix ordering.
    let forge_idx = built_in
        .iter()
        .position(|id| *id == ProviderId::FORGE)
        .unwrap();
    let openai_idx = built_in
        .iter()
        .position(|id| *id == ProviderId::OPENAI)
        .unwrap();
    let openrouter_idx = built_in
        .iter()
        .position(|id| *id == ProviderId::OPEN_ROUTER)
        .unwrap();
    let anthropic_idx = built_in
        .iter()
        .position(|id| *id == ProviderId::ANTHROPIC)
        .unwrap();

    assert!(forge_idx < openai_idx);
    assert!(openai_idx < openrouter_idx);
    assert!(openrouter_idx < anthropic_idx);
}

// ---------------------------------------------------------------------------
// 6. ProviderResponse and ProviderType enums
// ---------------------------------------------------------------------------

#[test]
fn provider_response_variants_serialize() {
    let cases = [
        (ProviderResponse::OpenAI, "\"OpenAI\""),
        (ProviderResponse::OpenAIResponses, "\"OpenAIResponses\""),
        (ProviderResponse::Anthropic, "\"Anthropic\""),
        (ProviderResponse::Bedrock, "\"Bedrock\""),
        (ProviderResponse::Google, "\"Google\""),
        (ProviderResponse::OpenCode, "\"OpenCode\""),
    ];

    for (variant, expected_json) in cases {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, expected_json, "serialize mismatch for {variant:?}");
    }
}

#[test]
fn provider_response_json_roundtrip() {
    let variants = [
        ProviderResponse::OpenAI,
        ProviderResponse::OpenAIResponses,
        ProviderResponse::Anthropic,
        ProviderResponse::Bedrock,
        ProviderResponse::Google,
        ProviderResponse::OpenCode,
    ];

    for variant in &variants {
        let json = serde_json::to_string(variant).unwrap();
        let deserialized: ProviderResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(*variant, deserialized, "roundtrip failed for {variant:?}");
    }
}

#[test]
fn provider_type_default_is_llm() {
    assert_eq!(ProviderType::default(), ProviderType::Llm);
}

#[test]
fn provider_type_json_roundtrip() {
    let variants = [ProviderType::Llm, ProviderType::ContextEngine];

    for variant in &variants {
        let json = serde_json::to_string(variant).unwrap();
        let deserialized: ProviderType = serde_json::from_str(&json).unwrap();
        assert_eq!(*variant, deserialized, "roundtrip failed for {variant:?}");
    }
}

#[test]
fn provider_type_json_serializes_snake_case() {
    let json = serde_json::to_string(&ProviderType::ContextEngine).unwrap();
    assert_eq!(json, "\"context_engine\"");
}

// ---------------------------------------------------------------------------
// 7. Model struct construction
// ---------------------------------------------------------------------------

#[test]
fn model_new_sets_id() {
    let model = Model::new("gpt-4o");
    assert_eq!(model.id.as_str(), "gpt-4o");
}

#[test]
fn model_with_builder_methods() {
    let model = Model::new("claude-3-opus")
        .name("Claude 3 Opus".to_string())
        .description("Anthropic's most capable model".to_string())
        .context_length(200000u64)
        .tools_supported(true)
        .supports_reasoning(true);

    assert_eq!(model.id.as_str(), "claude-3-opus");
    assert_eq!(model.name.as_deref(), Some("Claude 3 Opus"));
    assert_eq!(
        model.description.as_deref(),
        Some("Anthropic's most capable model")
    );
    assert_eq!(model.context_length, Some(200000));
    assert_eq!(model.tools_supported, Some(true));
    assert_eq!(model.supports_reasoning, Some(true));
}

// ---------------------------------------------------------------------------
// 8. Display name uniqueness
// ---------------------------------------------------------------------------

#[test]
fn display_names_are_unique_across_built_ins() {
    let mut names: Vec<String> = ProviderId::built_in_providers()
        .iter()
        .map(|id| id.to_string())
        .collect();
    let original_len = names.len();
    names.sort();
    names.dedup();
    assert_eq!(
        names.len(),
        original_len,
        "duplicate display names detected among built-in providers"
    );
}

// ---------------------------------------------------------------------------
// 9. ProviderId equality and hashing
// ---------------------------------------------------------------------------

#[test]
fn provider_id_equality_is_by_value() {
    let a = ProviderId::from_str("openai").unwrap();
    let b = ProviderId::OPENAI;
    assert_eq!(a, b);
}

#[test]
fn provider_id_hash_consistency() {
    use std::collections::HashMap;

    let mut map = HashMap::new();
    map.insert(ProviderId::OPENAI, "openai-value");
    map.insert(ProviderId::ANTHROPIC, "anthropic-value");

    assert_eq!(map.get(&ProviderId::OPENAI), Some(&"openai-value"));
    assert_eq!(map.get(&ProviderId::ANTHROPIC), Some(&"anthropic-value"));

    // Lookup by constructed value.
    let lookup = ProviderId::from_str("openai").unwrap();
    assert_eq!(map.get(&lookup), Some(&"openai-value"));
}
