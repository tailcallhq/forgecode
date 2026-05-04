# Static Model List Support for forge.toml Configuration (v2)

## Objective

Enable users to define static model lists directly in `forge.toml` provider entries, in addition to the existing URL-based model discovery. This allows offline or air-gapped deployments to specify models without requiring API calls to fetch model lists.

**v2 Change**: Remove the requirement that the models endpoint must exist and return a value. The behavior should be:

1. If there is **no `models` endpoint** in the `[[providers]]` section AND there are `[[providers.models]]`, don't need to check the `models` endpoint at all
2. If there **is a `models` endpoint** in the `[[providers]]` AND there are `[[providers.models]]`, check the `models` endpoint but only fail with a **warning** if it errors

## Project Structure Summary

The implementation spans three key areas:

1. **Schema Layer** (`forge.schema.json`): JSON Schema definition for configuration validation
2. **Config Layer** (`crates/forge_config/src/config.rs`): Rust struct definitions with Serde deserialization
3. **Repository Layer** (`crates/forge_repo/src/provider/provider_repo.rs`): Internal model handling with `Models` enum

## Relevant Files

| File | Purpose | Current State |
|------|---------|---------------|
| `forge.schema.json:613-689` | `ProviderEntry` schema | `models` accepts only `string \| null` |
| `crates/forge_config/src/config.rs:69-99` | `ProviderEntry` struct | `models: Option<String>` |
| `crates/forge_repo/src/provider/provider_repo.rs:13-21` | Internal `Models` enum | Already supports `Url` and `Hardcoded` |
| `crates/forge_domain/src/model.rs:23-38` | Domain `Model` struct | Defines model fields |
| `crates/forge_repo/src/provider/provider.json` | Embedded provider configs | Uses hardcoded model arrays |
| `crates/forge_repo/src/provider/model_fetch.rs` | Model fetching logic | Currently requires models endpoint to succeed |

## Implementation Plan

### Phase 1: Schema Updates

- [x] **Task 1.1**: Update `forge.schema.json` to define a new `ModelEntry` schema

  - Define `ModelEntry` object with fields matching `forge_domain::Model`:
    - `id` (string, required): Model identifier
    - `name` (string, optional): Human-readable model name
    - `description` (string, optional): Model description
    - `context_length` (integer, optional): Maximum context window
    - `tools_supported` (boolean, optional): Tool use capability
    - `supports_parallel_tool_calls` (boolean, optional): Parallel tool calls support
    - `supports_reasoning` (boolean, optional): Reasoning support
    - `input_modalities` (array of strings, optional): Supported input types (`["text"]`, `["text", "image"]`)

  - Update `ProviderEntry.models` field to accept `oneOf`:
    - String (URL template for fetching models)
    - Array of `ModelEntry` objects (static model list)
    - Null (no model list)

### Phase 2: Config Layer Updates

- [x] **Task 2.1**: Create `StaticModelEntry` struct in `forge_config/src/config.rs`

  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Dummy)]
  pub struct StaticModelEntry {
      pub id: String,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub name: Option<String>,
      // ... other optional fields
  }
  ```

  - Follow existing patterns: use `#[serde(default, skip_serializing_if = "Option::is_none")]` for optional fields
  - Use `derive_setters::Setters` for builder pattern support
  - Add JSON Schema derive for schema generation

- [x] **Task 2.2**: Create `ProviderModels` enum with untagged deserialization

  ```rust
  #[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
  #[serde(untagged)]
  pub enum ProviderModels {
      Url(String),
      Static(Vec<StaticModelEntry>),
  }
  ```

  - Use `#[serde(untagged)]` for seamless backward compatibility with string URLs
  - Keep serialization compatible with both formats

- [x] **Task 2.3**: Update `ProviderEntry` struct to use new enum

  ```rust
  pub struct ProviderEntry {
      // ... existing fields ...
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub models: Option<ProviderModels>,
  }
  ```

### Phase 3: Repository Layer Updates

- [x] **Task 3.1**: Update `From<forge_config::ProviderEntry>` conversion in `provider_repo.rs`

  - Modify line 158 to handle both `ProviderModels::Url` and `ProviderModels::Static` variants
  - Map `Static` variant to existing `Models::Hardcoded` enum variant
  - Ensure proper conversion of `StaticModelEntry` to `forge_app::domain::Model`

- [x] **Task 3.2**: Verify existing `From<&ProviderConfig>` conversion (lines 165-192) works unchanged

  - Internal `Models` enum already supports both variants
  - Conversion logic handles `Models::Hardcoded` correctly

### Phase 4: Validation

- [x] **Task 4.1**: Add runtime validation for static model entries

  - Validate `id` field is non-empty
  - Validate `context_length` is positive if provided
  - Validate `input_modalities` contains valid values (`"text"`, `"image"`)

- [x] **Task 4.2**: Consider adding validation at config load time

  - Implement `Validate` trait or custom deserialization logic
  - Provide clear error messages for invalid configurations

### Phase 5: Model Endpoint Validation Relaxation (v2)

- [ ] **Task 5.1**: Update model fetching logic to handle missing endpoint gracefully

  - Modify `crates/forge_repo/src/provider/model_fetch.rs` to check if `models` URL is provided
  - If no `models` URL is provided and static models exist, skip endpoint fetch entirely
  - If `models` URL is provided but fails, log a **warning** instead of failing hard

- [ ] **Task 5.2**: Add logging for models endpoint scenarios

  - Log when skipping models endpoint fetch (static models present, no URL)
  - Log warning when models endpoint fails (with URL present)
  - Log info when models are fetched successfully from endpoint

- [ ] **Task 5.3**: Update error handling in provider initialization

  - Change `Result` return type to handle models endpoint failures gracefully
  - Use `tracing::warn!` for endpoint failures instead of `tracing::error!`
  - Ensure static models are still usable even if endpoint fails

### Phase 6: Testing Strategy

- [x] **Task 6.1**: Unit tests for `ProviderModels` deserialization

  ```rust
  #[test]
  fn test_provider_models_url() {
      let json = r#""https://api.example.com/models""#;
      let models: ProviderModels = serde_json::from_str(json).unwrap();
      assert!(matches!(models, ProviderModels::Url(_)));
  }

  #[test]
  fn test_provider_models_static() {
      let json = r#"[{"id": "gpt-4"}]"#;
      let models: ProviderModels = serde_json::from_str(json).unwrap();
      assert!(matches!(models, ProviderModels::Static(_)));
  }
  ```

- [x] **Task 6.2**: Integration tests for full provider config parsing

  - Test inline provider with URL-based models
  - Test inline provider with static model list
  - Test mixed providers in single `ForgeConfig`

- [x] **Task 6.3**: Round-trip serialization tests

  - Verify static model lists serialize back to valid JSON
  - Verify URL strings remain unchanged

- [x] **Task 6.4**: Backward compatibility tests

  - Verify existing URL-only configurations continue to work
  - Test with various URL template formats

- [ ] **Task 6.5**: New tests for models endpoint relaxation

  - Test static models work without any `models` URL
  - Test static models still work when `models` URL fails (warning only)
  - Test warning is logged when endpoint fails
  - Test info is logged when endpoint succeeds

### Phase 7: Documentation

- [x] **Task 7.1**: Update schema documentation

  - Document both `models` format options in schema descriptions
  - Add examples showing URL vs static list usage

- [x] **Task 7.2**: Consider updating user-facing docs (optional)

  - Explain when to use URL-based vs static model lists
  - Provide example configurations

## Verification Criteria

- [x] Existing URL-based model configurations continue to work without modification
- [x] New static model list configurations parse correctly from TOML
- [x] Schema validation passes for both format options
- [x] All existing tests pass (`cargo insta test --accept`)
- [x] Code compiles without warnings (`cargo check`)
- [x] Round-trip serialization preserves model data accurately
- [ ] Static models work correctly when no `models` URL is provided
- [ ] Models endpoint failures produce warnings, not errors (when URL is present)
- [ ] All new tests pass

## Potential Risks and Mitigations

1. **Risk**: Breaking change for existing configurations
   - **Mitigation**: Untagged enum ensures backward compatibility with string URLs
   - **Mitigation**: Extensive testing of existing URL-based configs

2. **Risk**: Schema validation conflicts between URL and object formats
   - **Mitigation**: Use `oneOf` in JSON Schema with clear type discrimination
   - **Mitigation**: Rust untagged enum handles both cases automatically

3. **Risk**: Inconsistent model field validation
   - **Mitigation**: Align static model entry schema with domain `Model` struct fields
   - **Mitigation**: Add runtime validation for required fields

4. **Risk**: Merge semantics with hardcoded vs URL models
   - **Mitigation**: Existing merge logic in `provider_repo.rs` already handles `Models` enum
   - **Mitigation**: Verify merge behavior with both model source types

5. **Risk** (v2): Models endpoint errors go unnoticed
   - **Mitigation**: Ensure warnings are logged clearly with provider and endpoint context
   - **Mitigation**: Document this behavior in configuration docs

## Alternative Approaches

1. **Separate Fields Approach**: Use `models_url` and `models_static` as separate fields
   - **Trade-off**: More explicit but breaks compatibility
   - **Trade-off**: Users must choose one or the other

2. **Custom Deserializer Approach**: Keep string field but parse array syntax internally
   - **Trade-off**: Simpler schema but less explicit
   - **Trade-off**: Magic parsing could be confusing

3. **Reuse Existing Domain Types**: Reference domain `Model` type directly in config
   - **Trade-off**: DRY but couples config layer to domain layer
   - **Trade-off**: May introduce unnecessary dependencies

**Selected Approach**: Untagged enum with dedicated `StaticModelEntry` struct
- Preserves backward compatibility (URL strings deserialize directly)
- Clear separation of concerns (config vs domain)
- Follows existing codebase patterns (similar to `UrlParamVarConfig`)

## Implementation Order

1. ~~Update `forge.schema.json` first (schema drives validation)~~
2. ~~Implement `StaticModelEntry` and `ProviderModels` in `forge_config`~~
3. ~~Update `ProviderEntry` struct~~
4. ~~Update conversion logic in `provider_repo.rs`~~
5. ~~Add tests~~ (phases 1-4 complete)
6. **Implement models endpoint relaxation** (Phase 5 - new in v2)
7. **Add new tests for endpoint relaxation** (Phase 6.5 - new in v2)
8. Verify with `cargo check` and `cargo insta test`

## v2 Changelog

| Date | Change |
|------|--------|
| 2026-04-21 | Initial v2 - Added Phase 5 (Model Endpoint Validation Relaxation) and Phase 6.5 (new tests), updated verification criteria and risks |
