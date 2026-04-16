# General Index

## Root

- `Cargo.toml` - Workspace Cargo manifest with dependency and build configuration [BUILD]
- `package.json` - Node/TypeScript evals and benchmark scripts config [CONFIG]

## .forge/skills/create-plan/

- `validate-all-plans.sh` - Bash script to run plan validator across all plan files and summarize results [CLI]
- `validate-plan.sh` - Validates a single plan markdown file for structure and content quality [CLI]

## benchmarks/

- `cli.ts` - CLI entrypoint for running evaluation tasks: reads task.yml, generates contexts, runs commands in parallel, validates outputs, and emits summary/logs.. Key: `main`, `logger`, `execAsync`, `TaskResult`, `__dirname` [CLI]
- `command-generator.ts` - Utility functions to parse CSV-like input, produce cross-product contexts, and render Handlebars command templates.. Key: `loadCsvData`, `createCrossProduct`, `generateCommand`, `generateCommands`, `getContextsFromSources` [SOURCE_CODE]
- `model.ts` - TypeScript type definitions for benchmark tasks, validations, sources, and task status enums. Key: `Task`, `Validation`, `Source`, `TaskStatus` [SOURCE_CODE]
- `parse.ts` - CLI argument parsing and path resolution for benchmark evaluations. Key: `CliArgs`, `parseCliArgs` [CLI]
- `task-executor.ts` - Runs a single benchmark task command, streams stdout/stderr to a log, enforces timeout and optional early-exit validations, and returns a structured result.. Key: `TaskExecutionResult`, `executeTask` [SOURCE_CODE]
- `utils.ts` - Small TypeScript utility helpers used by benchmark scripts (timestamp formatting, regex escaping, temp dir, CSV parsing).. Key: `formatTimestamp`, `escapeRegex`, `createTempDir`, `parseCsvAsync` [SOURCE_CODE]

## benchmarks/evals/semantic_search_quality/

- `llm_judge.ts` - TS script that uses Gemini to judge semantic search query and result quality [CLI]
- `run_eval.sh` - Shell orchestration to verify sem_search usage and optionally run LLM judge [CLI]
- `run_tests.sh` - Test harness that runs multiple semantic search eval scenarios using run_eval.sh [CLI]

## crates/forge_api/src/

- `api.rs` - Defines the async API trait surface used by the rest of the application and external callers. Key: `API` [SOURCE_CODE]
- `forge_api.rs` - Concrete Forge API implementation that bridges app services and environment infra to the API trait.. Key: `ForgeAPI`, `new`, `init`, `app`, `get_skills_internal` [SOURCE_CODE]
- `lib.rs` - API crate re-exporting application DTOs and domain types. Key: `pub use api::* / pub use forge_api::* / pub use forge_app::dto::*` [SOURCE_CODE]

## crates/forge_app/src/

- `agent.rs` - Agent service trait and configuration application utilities. Key: `AgentService`, `impl<T: Services + EnvironmentInfra> AgentService for T`, `AgentExt`, `AgentExt::apply_config` [SOURCE_CODE]
- `agent_executor.rs` - Executes agents as tools and provides agent tool definitions. Key: `AgentExecutor`, `AgentExecutor::agent_definitions`, `AgentExecutor::execute`, `AgentExecutor::contains_tool` [SOURCE_CODE]
- `agent_provider_resolver.rs` - Resolves provider and model selection for agents, with credential handling. Key: `AgentProviderResolver`, `AgentProviderResolver::get_provider`, `AgentProviderResolver::get_model` [SOURCE_CODE]
- `app.rs` - Application-layer orchestrator that runs chat flows, compaction, and model/tool queries.. Key: `build_template_config`, `ForgeApp`, `new`, `chat`, `compact_conversation` [SOURCE_CODE]
- `apply_tunable_parameters.rs` - Applies agent-configured tunable parameters into a conversation context. Key: `ApplyTunableParameters`, `ApplyTunableParameters::new`, `ApplyTunableParameters::apply` [SOURCE_CODE]
- `changed_files.rs` - Detects externally modified files and injects notifications. Key: `ChangedFiles`, `ChangedFiles::update_file_stats`, `FileChangeDetector` [SOURCE_CODE]
- `command_generator.rs` - Generates a single-shell command from a natural language prompt via an LLM and JSON-schema response.. Key: `ShellCommandResponse`, `CommandGenerator`, `CommandGenerator::generate`, `CommandGenerator::create_context` [SOURCE_CODE]
- `compact.rs` - Context compaction service: summarize and compress assistant message sequences while preserving usage and reasoning continuity.. Key: `Compactor`, `transform`, `compact`, `compress_single_sequence` [SOURCE_CODE]
- `data_gen.rs` - Data generation app that streams model-driven JSON outputs from schema and templates. Key: `DataGenerationApp`, `read_file`, `load_parameters`, `execute` [SOURCE_CODE]
- `error.rs` - Application-level error enum for tool and agent runtime errors. Key: `Error` [SOURCE_CODE]
- `file_tracking.rs` - Detects external file changes by comparing stored and current hashes. Key: `FileChange`, `FileChangeDetector`, `FileChangeDetector::detect`, `MockFsReadService` [SOURCE_CODE]
- `git_app.rs` - Implements git-related flows: fetching diffs/context, generating commit messages via LLM, and performing commits.. Key: `GitApp`, `GitAppError`, `CommitResult`, `CommitMessageDetails`, `CommitMessageResponse` [SOURCE_CODE]
- `infra.rs` - Trait-based infrastructure abstraction for filesystem, env, HTTP, OAuth, MCP, gRPC and user I/O. Key: `EnvironmentInfra`, `FileReaderInfra`, `FileWriterInfra`, `FileRemoverInfra`, `FileInfoInfra` [SOURCE_CODE]
- `init_conversation_metrics.rs` - Initialize conversation metrics with a start timestamp. Key: `InitConversationMetrics`, `InitConversationMetrics::apply`, `test_sets_started_at` [SOURCE_CODE]
- `lib.rs` - Top-level reexports and module declarations for the application crate. Key: `agent`, `services`, `utils::compute_hash`, `domain` [SOURCE_CODE]
- `mcp_executor.rs` - Executor that forwards MCP tool calls to registered MCP services. Key: `McpExecutor<S>`, `McpExecutor::execute`, `McpExecutor::contains_tool` [SOURCE_CODE]
- `operation.rs` - Represents tool operations and translates them into user-facing ToolOutput with truncation and metrics bookkeeping.. Key: `TempContentFiles`, `ToolOperation`, `StreamElement`, `create_stream_element`, `create_validation_warning` [SOURCE_CODE]
- `orch.rs` - Orchestrates an agent turn/loop: chat calls, tool execution, lifecycle events, and streaming.. Key: `Orchestrator`, `new`, `get_conversation`, `execute_tool_calls`, `send` [SOURCE_CODE]
- `retry.rs` - Retry helper that applies backoff strategy for retryable errors. Key: `retry_with_config`, `should_retry` [SOURCE_CODE]
- `search_dedup.rs` - Deduplicate semantic code search results across queries. Key: `Score`, `Score::new`, `deduplicate_results` [SOURCE_CODE]
- `services.rs` - Central collection of service interface traits and small DTOs used across the application. Key: `ShellOutput`, `PatchOutput`, `ReadOutput`, `Content`, `SearchResult` [SOURCE_CODE]
- `set_conversation_id.rs` - Populate conversation context with its own ID. Key: `SetConversationId`, `SetConversationId::apply`, `test_sets_conversation_id` [SOURCE_CODE]
- `system_prompt.rs` - Builds system prompt blocks using templates, files, tools and git stats. Key: `SystemPrompt`, `SystemPrompt::add_system_message`, `SystemPrompt::fetch_extensions`, `parse_extensions`, `SystemPrompt::is_tool_supported` [SOURCE_CODE]
- `template_engine.rs` - Handlebars-based template engine with custom helpers and embedded templates. Key: `create_handlebar`, `HANDLEBARS`, `TemplateEngine` [SOURCE_CODE]
- `title_generator.rs` - Generates conversation titles using an LLM with structured JSON schema output. Key: `TitleResponse`, `TitleGenerator`, `TitleGenerator::generate` [SOURCE_CODE]
- `tool_executor.rs` - Executes tool calls by routing ToolCatalog inputs to services, enforcing policies and producing ToolOutput.. Key: `ToolExecutor`, `require_prior_read`, `normalize_path`, `create_temp_file`, `call_internal` [SOURCE_CODE]
- `tool_registry.rs` - Registry and dispatcher for executing tools (Forge tools, delegated agents, MCP tools) with permission, modality and timeout checks. Key: `ToolRegistry`, `ToolRegistry::new`, `ToolRegistry::call`, `ToolRegistry::call_inner`, `ToolRegistry::call_with_timeout` [SOURCE_CODE]
- `tool_resolver.rs` - Resolves and filters available tool definitions for agents (supports globs & aliases). Key: `ToolResolver`, `deprecated_tool_aliases`, `resolve`, `is_allowed` [SOURCE_CODE]
- `user.rs` - User, plan and usage domain types for auth/plan data. Key: `AuthProviderId`, `User`, `Plan::is_upgradeable`, `UsageInfo` [SOURCE_CODE]
- `user_prompt.rs` - Generates and injects user prompts and attachments into conversations. Key: `UserPromptGenerator`, `UserPromptGenerator::add_user_prompt`, `UserPromptGenerator::add_rendered_message`, `UserPromptGenerator::add_attachments`, `UserPromptGenerator::add_todos_on_resume` [SOURCE_CODE]
- `utils.rs` - General-purpose utilities: path formatting, hashing and schema normalization. Key: `format_display_path`, `format_match`, `compute_hash`, `enforce_strict_schema`, `is_binary_content_type` [SOURCE_CODE]
- `walker.rs` - Filesystem walker configuration and walked file representation. Key: `Walker`, `Walker::conservative`, `WalkedFile`, `WalkedFile::is_dir` [SOURCE_CODE]
- `workspace_status.rs` - Compute file sync status and sync operation paths against remote hashes. Key: `WorkspaceStatus`, `WorkspaceStatus::file_statuses`, `WorkspaceStatus::get_sync_paths`, `SyncProgressCounter`, `absolutize` [SOURCE_CODE]

## crates/forge_app/src/dto/

- `mod.rs` - Top-level DTO module that namespaces provider DTOs. Key: `anthropic`, `google`, `openai`, `tools_overview` [SOURCE_CODE]

## crates/forge_app/src/dto/anthropic/

- `request.rs` - DTOs and serialization for Anthropic request payloads. Key: `Request`, `TryFrom<forge_domain::Context> for Request`, `Message`, `Content` [SOURCE_CODE]
- `response.rs` - Parsing and mapping Anthropic streaming responses to domain messages. Key: `Event / EventData`, `ContentBlock`, `Usage`, `impl TryFrom<EventData> for ChatCompletionMessage`, `get_context_length` [SOURCE_CODE]

## crates/forge_app/src/dto/openai/

- `request.rs` - DTOs and conversions for OpenAI-style request payloads. Key: `Request`, `Message`, `ContentPart`, `impl From<Context> for Request`, `serialize_tool_call_arguments` [SOURCE_CODE]
- `response.rs` - Parses provider responses and converts them into internal chat messages. Key: `Response`, `Choice`, `ResponseUsage`, `ToolCall`, `impl TryFrom<Response> for ChatCompletionMessage` [SOURCE_CODE]

## crates/forge_app/src/dto/openai/transformers/

- `drop_tool_call.rs` - Transformer that removes tool call metadata for providers without tool support. Key: `DropToolCalls`, `Transformer::transform (impl for DropToolCalls)`, `test_mistral_transformer_tools_not_supported` [SOURCE_CODE]
- `mod.rs` - Module re-exporting OpenAI provider transformation pipeline components. Key: `ProviderPipeline` [SOURCE_CODE]
- `pipeline.rs` - Provider-specific request transformation pipeline. Key: `ProviderPipeline`, `is_zai_provider`, `is_gemini3_model`, `supports_open_router_params` [SOURCE_CODE]

## crates/forge_app/src/fmt/

- `fmt_input.rs` - Converts ToolCatalog inputs into user-facing ChatResponseContent titles/subtitles for CLI/TUI display. Key: `FormatContent`, `ToolCatalog`, `to_content` [SOURCE_CODE]
- `fmt_output.rs` - Converts ToolOperation variants into optional ChatResponseContent for UI output (diffs, titles, todo text); includes unit tests.. Key: `to_content`, `FormatContent`, `tests`, `fixture_environment` [SOURCE_CODE]
- `todo_fmt.rs` - Formats Todo lists/diffs into ANSI-styled checklist output. Key: `format_todo_line`, `format_todos_diff`, `format_todos` [SOURCE_CODE]

## crates/forge_app/src/hooks/

- `compaction.rs` - Event hook that triggers context compaction when thresholds are exceeded. Key: `CompactionHandler`, `CompactionHandler::new`, `EventHandle<EventData<ResponsePayload>> for CompactionHandler::handle` [SOURCE_CODE]
- `doom_loop.rs` - Detects repeating tool-call doom loops in conversation history. Key: `DoomLoopDetector`, `DoomLoopDetector::detect_from_conversation`, `DoomLoopDetector::check_repeating_pattern`, `EventHandle<EventData<RequestPayload>> for DoomLoopDetector` [SOURCE_CODE]
- `mod.rs` - Re-exports hook handlers used by the application. Key: `CompactionHandler`, `DoomLoopDetector`, `TitleGenerationHandler`, `TracingHandler` [SOURCE_CODE]
- `title_generation.rs` - Asynchronous per-conversation title generation hook using background tasks. Key: `TitleGenerationHandler`, `TitleTask`, `impl EventHandle<EventData<StartPayload>> for TitleGenerationHandler`, `impl EventHandle<EventData<EndPayload>> for TitleGenerationHandler` [SOURCE_CODE]
- `tracing.rs` - Logging/tracing handler for conversation lifecycle events. Key: `TracingHandler`, `EventHandle<EventData<ResponsePayload>> for TracingHandler`, `EventHandle<EventData<ToolcallEndPayload>> for TracingHandler` [SOURCE_CODE]

## crates/forge_app/src/orch_spec/

- `orch_setup.rs` - Test harness types and default fixtures used to run orchestrator integration-style tests.. Key: `TestContext`, `Default for TestContext`, `run`, `TestOutput` [SOURCE_CODE]

## crates/forge_app/src/transformers/

- `strip_working_dir.rs` - Transformer that strips working-dir prefixes from file paths in summaries. Key: `StripWorkingDir`, `StripWorkingDir::new`, `StripWorkingDir::strip_prefix`, `Transformer::transform for StripWorkingDir` [SOURCE_CODE]
- `trim_context_summary.rs` - Transformer that deduplicates redundant assistant operations in summaries. Key: `TrimContextSummary`, `Operation`, `to_op` [SOURCE_CODE]

## crates/forge_ci/src/

- `release_matrix.rs` - Defines CI release build matrix entries and JSON conversion. Key: `MatrixEntry`, `ReleaseMatrix`, `impl From<ReleaseMatrix> for Value` [SOURCE_CODE]

## crates/forge_ci/src/jobs/

- `bounty_job.rs` - Defines CI job steps for bounty label synchronization scripts [BUILD]
- `draft_release_update_job.rs` - GitHub Actions job to update the release draft via release-drafter [BUILD]
- `mod.rs` - Re-exports CI job modules for workflow generation [BUILD]
- `release_build_job.rs` - Builder for matrix release build job (cross/target builds) [BUILD]
- `release_draft.rs` - Generates a GitHub Actions job to create a draft release [BUILD]

## crates/forge_ci/src/steps/

- `setup_protoc.rs` - Reusable CI step to install the Protobuf compiler [BUILD]

## crates/forge_ci/src/workflows/

- `autofix.rs` - Generates GitHub Actions workflow to autofix formatting and lint issues. Key: `generate_autofix_workflow`, `lint_fix_job` [SOURCE_CODE]
- `bounty.rs` - Generates the bounty management GitHub Actions workflow [BUILD]
- `ci.rs` - Generates the project's GitHub Actions CI workflow [BUILD]
- `mod.rs` - Re-exports CI workflow definitions [BUILD]

## crates/forge_config/src/

- `auto_dump.rs` - Defines conversation auto-dump output formats. Key: `AutoDumpFormat` [SOURCE_CODE]
- `compact.rs` - Configuration structures for compaction and update frequency. Key: `UpdateFrequency`, `Update`, `Compact` [SOURCE_CODE]
- `config.rs` - Defines the top-level ForgeConfig schema and provider configuration structures with read/write helpers.. Key: `ProviderResponseType`, `ProviderTypeEntry`, `ProviderAuthMethod`, `ProviderUrlParam`, `ProviderEntry` [SOURCE_CODE]
- `decimal.rs` - Decimal newtype that serializes to two decimal places for clean TOML output. Key: `Decimal`, `impl serde::Serialize/Deserialize for Decimal`, `impl schemars::JsonSchema for Decimal` [SOURCE_CODE]
- `error.rs` - Defines configuration-related error types. Key: `Error` [SOURCE_CODE]
- `http.rs` - HTTP client configuration types (TLS versions/backends and timeouts). Key: `TlsVersion`, `TlsBackend`, `HttpConfig` [SOURCE_CODE]
- `legacy.rs` - Converts legacy JSON config to the new TOML ForgeConfig representation. Key: `LegacyConfig`, `LegacyConfig::read`, `LegacyConfig::into_forge_config` [SOURCE_CODE]
- `lib.rs` - Top-level re-exports and Result alias for config crate. Key: `Result`, `pub use config::*` [SOURCE_CODE]
- `model.rs` - Model and provider identifier types and ModelConfig pairing. Key: `ProviderId`, `ModelId`, `ModelConfig` [SOURCE_CODE]
- `percentage.rs` - Validated percentage type constrained to [0.0,1.0] with two-decimal serialization. Key: `Percentage`, `Percentage::new`, `serde impl for Percentage` [SOURCE_CODE]
- `reader.rs` - Layered Forge configuration reader with env and legacy support. Key: `LOAD_DOT_ENV`, `ConfigReader`, `ConfigReader::build`, `ConfigReader::read_env` [SOURCE_CODE]
- `reasoning.rs` - Configuration types for model reasoning behavior. Key: `ReasoningConfig`, `Effort` [SOURCE_CODE]
- `retry.rs` - Retry/backoff configuration struct. Key: `RetryConfig` [SOURCE_CODE]
- `writer.rs` - Writes ForgeConfig to disk with schema header for editor validation. Key: `ConfigWriter`, `ConfigWriter::write` [SOURCE_CODE]

## crates/forge_display/src/

- `code.rs` - Markdown code-block extraction and terminal syntax highlighting. Key: `SyntaxHighlighter`, `CodeBlock`, `CodeBlockParser`, `CodeBlockParser::new`, `CodeBlockParser::restore` [SOURCE_CODE]
- `diff.rs` - Generate colored inline diffs with line numbers and counts. Key: `DiffFormat::format`, `DiffResult`, `Line` [SOURCE_CODE]
- `grep.rs` - Format ripgrep-style search results with optional regex highlighting. Key: `GrepFormat`, `ParsedLine::parse`, `GrepFormat::format` [SOURCE_CODE]
- `lib.rs` - Display formats re-exports for diff, grep and markdown. Key: `DiffFormat`, `GrepFormat`, `MarkdownFormat` [SOURCE_CODE]
- `markdown.rs` - Render markdown for terminal with syntax highlighting. Key: `MarkdownFormat`, `MarkdownFormat::render`, `MarkdownFormat::strip_excessive_newlines` [SOURCE_CODE]

## crates/forge_domain/src/

- `agent.rs` - Domain model definitions for Agent and related reasoning configuration. Key: `AgentId`, `Agent`, `ReasoningConfig`, `Effort`, `estimate_token_count` [SOURCE_CODE]
- `attachment.rs` - Attachment types and parser for file/directory references in chat messages. Key: `Attachment`, `AttachmentContent`, `Attachment::parse_all`, `FileTag`, `FileTag::parse` [SOURCE_CODE]
- `chat_request.rs` - ChatRequest wrapper for events tied to a conversation. Key: `ChatRequest`, `ChatRequest::new` [SOURCE_CODE]
- `chat_response.rs` - Domain types for agent chat responses and tool call events. Key: `ChatResponseContent`, `ChatResponse`, `TitleFormat`, `InterruptionReason`, `Cause` [SOURCE_CODE]
- `command.rs` - Domain struct for user-defined markdown-backed commands. Key: `Command` [SOURCE_CODE]
- `console.rs` - Trait for synchronized console output writers. Key: `ConsoleWriter` [SOURCE_CODE]
- `context.rs` - Domain types representing messages, contexts, and related utilities. Key: `ContextMessage`, `TextMessage`, `Context`, `token_count_approx`, `MessageEntry` [SOURCE_CODE]
- `conversation.rs` - Domain Conversation model, IDs and helpers. Key: `ConversationId`, `Conversation`, `MetaData`, `Conversation::to_html`, `Conversation::related_conversation_ids` [SOURCE_CODE]
- `conversation_html.rs` - HTML renderer for Conversation objects. Key: `render_conversation_html`, `render_conversation_html_with_related`, `create_info_table`, `create_conversation_context_section`, `create_tools_section` [SOURCE_CODE]
- `data_gen.rs` - Parameters for LLM-driven data generation jobs. Key: `DataGenerationParameters` [SOURCE_CODE]
- `env.rs` - Domain-level environment and session configuration helpers plus path helpers for runtime artifacts.. Key: `SessionConfig`, `ConfigOperation`, `VERSION`, `Environment`, `history_path` [SOURCE_CODE]
- `error.rs` - Domain error enum and helpers. Key: `Error`, `Result`, `ToolCallArgumentError` [SOURCE_CODE]
- `event.rs` - Event and command structures used for CLI event dispatch. Key: `Event`, `EventValue`, `UserCommand`, `EventContext`, `UserPrompt` [SOURCE_CODE]
- `file.rs` - Domain models for file metadata, hashes and sync statuses. Key: `File`, `FileInfo`, `FileHash`, `SyncStatus`, `FileStatus` [SOURCE_CODE]
- `file_operation.rs` - Record metrics for file operations performed by tools. Key: `FileOperation`, `FileOperation::new` [SOURCE_CODE]
- `fuzzy_search.rs` - Represents ranges matched by fuzzy search. Key: `SearchMatch` [SOURCE_CODE]
- `group_by_key.rs` - Utility trait to group collections by a computed key. Key: `GroupByKey`, `impl GroupByKey for Vec<V>::group_by_key` [SOURCE_CODE]
- `hook.rs` - Lifecycle hook/event system for conversation processing. Key: `EventData`, `LifecycleEvent`, `EventHandle`, `Hook` [SOURCE_CODE]
- `http_config.rs` - HTTP client configuration including TLS and HTTP/2 options. Key: `TlsVersion`, `TlsBackend`, `HttpConfig` [SOURCE_CODE]
- `image.rs` - Image data URL helper for base64-encoded images. Key: `Image`, `Image::new_bytes`, `Image::data` [SOURCE_CODE]
- `lib.rs` - Crate root that declares domain modules and re-exports the entire domain API for other crates.. Key: `ArcSender`, `line_numbers`, `fuzzy_search` [SOURCE_CODE]
- `line_numbers.rs` - Utility to number lines of text for display. Key: `NumberedContent`, `LineNumbers` [SOURCE_CODE]
- `max_tokens.rs` - Validated max_tokens newtype with serde support. Key: `MaxTokens`, `MaxTokens::new`, `impl Serialize/Deserialize for MaxTokens` [SOURCE_CODE]
- `mcp.rs` - Model Context Protocol (MCP) server configuration types. Key: `McpServerConfig`, `McpStdioServer`, `McpHttpServer`, `McpConfig`, `ServerName` [SOURCE_CODE]
- `mcp_servers.rs` - Cache structure for MCP servers and their tool definitions. Key: `McpServers`, `McpServers::new`, `impl IntoIterator for McpServers` [SOURCE_CODE]
- `merge.rs` - Helpers for merging/overwriting collections and options. Key: `std::overwrite`, `vec::unify_by_key`, `option`, `Key` [SOURCE_CODE]
- `message.rs` - Message and usage domain models for LLM chat responses. Key: `MessagePhase`, `Usage`, `ChatCompletionMessage`, `Content` [SOURCE_CODE]
- `message_pattern.rs` - Test helper that builds Context message sequences from compact patterns. Key: `MessagePattern`, `MessagePattern::new`, `MessagePattern::build`, `From<&str> for MessagePattern`, `tests` [SOURCE_CODE]
- `migration.rs` - Result type for credential migration from env to file. Key: `MigrationResult`, `MigrationResult::new`, `test_migration_result` [SOURCE_CODE]
- `model.rs` - Model metadata types and model identifier wrapper. Key: `InputModality`, `Model`, `Parameters`, `ModelId` [SOURCE_CODE]
- `model_config.rs` - Pairs provider and model identifiers into a config object. Key: `ModelConfig`, `ModelConfig::new` [SOURCE_CODE]
- `node.rs` - Domain types for workspace indexing, code search nodes, and sync progress events. Key: `SyncProgress`, `WorkspaceAuth`, `FileRead`, `CodeBase`, `SearchParams` [SOURCE_CODE]
- `point.rs` - Domain model for embedding points and search queries. Key: `PointId`, `Point`, `Point::new`, `Point::try_map`, `Query` [SOURCE_CODE]
- `provider.rs` - Domain types and helpers representing external model providers, their IDs, URLs, models and credentials. Key: `ProviderType`, `ProviderId`, `ProviderResponse`, `ModelSource`, `Provider` [SOURCE_CODE]
- `reasoning.rs` - Aggregate streaming reasoning parts into full reasoning entries. Key: `ReasoningDetail`, `Reasoning`, `Reasoning::from_parts` [SOURCE_CODE]
- `repo.rs` - Trait definitions for repository interfaces covering snapshots, conversations, providers, workspace indexing, validation and fuzzy search. Key: `SnapshotRepository`, `ConversationRepository`, `ChatRepository`, `ProviderRepository`, `WorkspaceIndexRepository` [SOURCE_CODE]
- `session_metrics.rs` - Session metrics storage for file operations, todos and timing. Key: `Metrics`, `Metrics::insert`, `Metrics::apply_todo_changes`, `Metrics::get_active_todos` [SOURCE_CODE]
- `shell.rs` - Represents command execution output. Key: `CommandOutput`, `CommandOutput::success` [SOURCE_CODE]
- `skill.rs` - Represents reusable skills/prompts and their metadata. Key: `Skill`, `Skill::new` [SOURCE_CODE]
- `snapshot.rs` - File snapshot metadata and path generation utilities. Key: `SnapshotId`, `Snapshot::create`, `Snapshot::path_hash`, `Snapshot::snapshot_path`, `Snapshot` [SOURCE_CODE]
- `suggestion.rs` - Simple struct modeling a usage suggestion. Key: `Suggestion` [SOURCE_CODE]
- `system_context.rs` - Structures for system prompt context and extension stats. Key: `ExtensionStat`, `Extension`, `TemplateConfig`, `SystemContext` [SOURCE_CODE]
- `temperature.rs` - Validated temperature newtype for model randomness configuration. Key: `Temperature`, `Temperature::new`, `Temperature::new_unchecked`, `Serialize/Deserialize impls for Temperature` [SOURCE_CODE]
- `template.rs` - Generic template wrapper with JSON schema support. Key: `Template<V>`, `Template::new`, `impl JsonSchema for Template<T>`, `From<S> for Template<Value>` [SOURCE_CODE]
- `tool_order.rs` - Ordering and pattern-based prioritization for tools. Key: `ToolOrder`, `ToolOrder::new`, `ToolOrder::sort`, `ToolOrder::get_weight`, `ToolOrder::compare_by_weight` [SOURCE_CODE]
- `top_k.rs` - Validated top_k newtype for model token filtering. Key: `TopK`, `TopK::new`, `Serialize/Deserialize impls for TopK` [SOURCE_CODE]
- `top_p.rs` - Validated top_p newtype for nucleus sampling configuration. Key: `TopP`, `TopP::new`, `Serialize/Deserialize impls for TopP` [SOURCE_CODE]
- `update.rs` - Update scheduling settings and frequency conversion. Key: `UpdateFrequency`, `impl From<UpdateFrequency> for Duration`, `Update` [SOURCE_CODE]
- `validation.rs` - Syntax error diagnostic struct. Key: `SyntaxError` [SOURCE_CODE]
- `workspace.rs` - Workspace UUID identifier type. Key: `WorkspaceId`, `WorkspaceId::generate`, `WorkspaceId::from_string` [SOURCE_CODE]
- `xml.rs` - Helpers to extract or remove XML-style tag content from text. Key: `extract_tag_content`, `remove_tag_with_prefix` [SOURCE_CODE]

## crates/forge_domain/src/auth/

- `auth_context.rs` - Types for managing various OAuth and API key auth flows. Key: `URLParameters`, `ApiKeyRequest`, `ApiKeyResponse`, `CodeRequest / CodeResponse`, `DeviceCodeRequest / DeviceCodeResponse` [SOURCE_CODE]
- `auth_method.rs` - Defines authentication method enum and utility accessors. Key: `AuthMethod`, `AuthMethod::oauth_config`, `tests::test_codex_device_deserializes_from_json` [SOURCE_CODE]
- `auth_token_response.rs` - Data structure representing OAuth token responses. Key: `OAuthTokenResponse`, `default_token_type` [SOURCE_CODE]
- `credentials.rs` - Defines credential types and utilities for provider authentication. Key: `AuthCredential`, `AuthDetails`, `OAuthTokens` [SOURCE_CODE]
- `new_types.rs` - Newtype wrappers for authentication-related string values. Key: `ApiKey`, `truncate_key`, `URLParamSpec` [SOURCE_CODE]

## crates/forge_domain/src/compact/

- `compact_config.rs` - Configuration and trigger logic for context compaction. Key: `Compact`, `Compact::should_compact`, `deserialize_percentage` [SOURCE_CODE]
- `strategy.rs` - Algorithms to compute which context messages to evict during compaction. Key: `CompactionStrategy`, `CompactionStrategy::to_fixed`, `CompactionStrategy::eviction_range`, `find_sequence_preserving_last_n` [SOURCE_CODE]

## crates/forge_domain/src/policies/

- `engine.rs` - Policy engine to evaluate permissions for operations. Key: `PolicyEngine`, `PolicyEngine::can_perform`, `PolicyEngine::evaluate_policies`, `PolicyEngine::evaluate_policy_set` [SOURCE_CODE]

## crates/forge_domain/src/tools/

- `catalog.rs` - Domain tool schema: ToolCatalog enum and tool input/output structs used to express agent tool calls.. Key: `ToolCatalog`, `SearchQuery`, `FSRead`, `FSWrite`, `Todo` [SOURCE_CODE]

## crates/forge_domain/src/tools/call/

- `context.rs` - Context wrapper used during tool call execution for metrics and messaging. Key: `ToolCallContext`, `ToolCallContext::new`, `ToolCallContext::send`, `ToolCallContext::with_metrics`, `ToolCallContext::update_todos` [SOURCE_CODE]
- `tool_call.rs` - Structures and utilities for assembling and tracking tool calls. Key: `ToolCallId`, `ToolCallPart`, `ToolCallFull`, `ToolCallFull::try_from_parts`, `ToolErrorTracker` [SOURCE_CODE]

## crates/forge_domain/src/transformer/

- `drop_reasoning_details.rs` - Transformer that strips reasoning details and reasoning config from context. Key: `DropReasoningDetails`, `transform` [SOURCE_CODE]
- `image_handling.rs` - Transformer that extracts images from tool outputs into separate image messages. Key: `ImageHandling`, `transform` [SOURCE_CODE]
- `mod.rs` - Generic transformer trait and composition utilities. Key: `Transformer`, `DefaultTransformation`, `Pipe`, `Cond` [SOURCE_CODE]
- `reasoning_normalizer.rs` - Transformer that strips assistant reasoning when model mismatch occurs. Key: `ReasoningNormalizer`, `ReasoningNormalizer::new`, `Transformer::transform for ReasoningNormalizer` [SOURCE_CODE]
- `set_model.rs` - Transformer that assigns a default model to text messages lacking one. Key: `SetModel`, `transform` [SOURCE_CODE]
- `transform_tool_calls.rs` - Transformer that flattens tool-supported messages into standard context messages. Key: `TransformToolCalls`, `TransformToolCalls::new`, `Transformer::transform for TransformToolCalls` [SOURCE_CODE]

## crates/forge_embed/src/

- `lib.rs` - Helpers to register embedded templates with Handlebars. Key: `files`, `register_templates` [SOURCE_CODE]

## crates/forge_fs/src/

- `binary_detection.rs` - Heuristic BOM and zero-byte based binary file detection. Key: `Encoding`, `Encoding::detect`, `is_binary`, `is_binary_internal` [SOURCE_CODE]
- `error.rs` - Filesystem-related error types for ForgeFS. Key: `Error` [SOURCE_CODE]
- `file_size.rs` - Async helper to get file size from metadata. Key: `ForgeFS::file_size` [SOURCE_CODE]
- `is_binary.rs` - File type detection using infer crate and sample reads. Key: `ForgeFS::is_binary_path`, `ForgeFS::is_binary` [SOURCE_CODE]
- `lib.rs` - Filesystem abstraction and utilities for uniform error handling. Key: `ForgeFS`, `ForgeFS::compute_hash`, `is_binary`, `Error` [SOURCE_CODE]
- `meta.rs` - Filesystem metadata and simple helpers on ForgeFS. Key: `ForgeFS::exists`, `ForgeFS::is_binary_file`, `ForgeFS::is_file`, `ForgeFS::read_dir` [SOURCE_CODE]
- `read.rs` - File reading helpers (bytes and UTF-8 variants) for ForgeFS. Key: `ForgeFS::read`, `ForgeFS::read_utf8`, `ForgeFS::read_to_string` [SOURCE_CODE]
- `read_range.rs` - Read specific line ranges from files with UTF-8 handling. Key: `ForgeFS::read_range_utf8`, `tests` [SOURCE_CODE]
- `write.rs` - Async file system write helpers on ForgeFS. Key: `ForgeFS::create_dir_all`, `ForgeFS::write`, `ForgeFS::append`, `ForgeFS::remove_file` [SOURCE_CODE]

## crates/forge_infra/src/

- `console.rs` - Thread-safe console writer synchronizing stdout/stderr writes. Key: `StdConsoleWriter`, `StdConsoleWriter::with_writers`, `impl ConsoleWriter for StdConsoleWriter`, `tests` [SOURCE_CODE]
- `env.rs` - Infra layer that constructs domain Environment, provides cached ForgeConfig, and applies ConfigOperation mutations to persisted ForgeConfig.. Key: `to_environment`, `apply_config_op`, `ForgeEnvironmentInfra`, `cached_config` [SOURCE_CODE]
- `error.rs` - Infrastructure error types for MCP integration. Key: `Error`, `UnsupportedMcpResponse` [SOURCE_CODE]
- `executor.rs` - Service to execute shell commands with streaming output. Key: `ForgeCommandExecutorService`, `execute_command_internal`, `OutputPrinterWriter`, `stream` [SOURCE_CODE]
- `forge_infra.rs` - Central infrastructure aggregator that implements infra traits for file, HTTP, gRPC, auth, commands, and user I/O.. Key: `ForgeInfra`, `new`, `config`, `read_batch_utf8`, `create_auth_strategy` [SOURCE_CODE]
- `fs_create_dirs.rs` - Filesystem directory-creation infra implementation. Key: `ForgeCreateDirsService`, `create_dirs` [SOURCE_CODE]
- `fs_meta.rs` - File metadata and existence checks service. Key: `ForgeFileMetaService`, `is_file`, `is_binary`, `exists`, `file_size` [SOURCE_CODE]
- `fs_read.rs` - File reader infra bridging ForgeFS into application infra traits. Key: `ForgeFileReadService`, `ForgeFileReadService::read_utf8`, `ForgeFileReadService::read_batch_utf8`, `tests` [SOURCE_CODE]
- `fs_read_dir.rs` - Directory reader service that lists and reads files with filtering. Key: `ForgeDirectoryReaderService`, `ForgeDirectoryReaderService::list_directory_entries`, `ForgeDirectoryReaderService::read_directory_files`, `tests` [SOURCE_CODE]
- `fs_remove.rs` - Low-level file removal infra service. Key: `ForgeFileRemoveService`, `new`, `remove` [SOURCE_CODE]
- `fs_write.rs` - Infrastructure service for writing files with parent-dir handling. Key: `ForgeFileWriteService`, `ForgeFileWriteService::create_parent_dirs`, `impl FileWriterInfra for ForgeFileWriteService` [SOURCE_CODE]
- `grpc.rs` - Lazily-initialized, shared gRPC Channel wrapper with optional TLS. Key: `ForgeGrpcClient`, `ForgeGrpcClient::new`, `ForgeGrpcClient::channel`, `ForgeGrpcClient::hydrate` [SOURCE_CODE]
- `http.rs` - HTTP client infra with debug request dumping and TLS config. Key: `ForgeHttpInfra`, `to_reqwest_tls`, `sanitize_headers`, `ForgeHttpInfra::execute_request`, `ForgeHttpInfra::write_debug_request` [SOURCE_CODE]
- `inquire.rs` - Async bridge to blocking terminal prompts and selections. Key: `ForgeInquire`, `ForgeInquire::prompt`, `UserInfra::prompt_question`, `UserInfra::select_one`, `UserInfra::select_many` [SOURCE_CODE]
- `kv_storage.rs` - cacache-backed generic key-value storage with TTL. Key: `CachedEntry`, `CacacheStorage`, `key_to_string`, `cache_get`, `cache_set` [SOURCE_CODE]
- `lib.rs` - Infrastructure crate re-exports and module declarations. Key: `StdConsoleWriter`, `ForgeEnvironmentInfra`, `ForgeCommandExecutorService`, `sanitize_headers`, `CacacheStorage` [SOURCE_CODE]
- `mcp_client.rs` - MCP client that connects to MCP servers (stdio/http/sse) and calls tools. Key: `ForgeMcpClient`, `ForgeMcpClient::create_connection`, `ForgeMcpClient::list`, `ForgeMcpClient::call`, `resolve_http_templates` [SOURCE_CODE]
- `mcp_server.rs` - MCP server connector producing MCP clients. Key: `ForgeMcpServer`, `connect` [SOURCE_CODE]
- `walker.rs` - Filesystem walker service that adapts Walker config and returns WalkedFile list. Key: `ForgeWalkerService`, `ForgeWalkerService::new`, `ForgeWalkerService::walk` [SOURCE_CODE]

## crates/forge_json_repair/src/

- `error.rs` - Error types for JSON repair/parsing. Key: `JsonRepairError`, `Result` [SOURCE_CODE]
- `lib.rs` - Public API re-exports for JSON repair utilities. Key: `JsonRepairError`, `json_repair`, `coerce_to_schema` [SOURCE_CODE]
- `parser.rs` - Robust parser that repairs and deserializes malformed JSON-like text. Key: `JsonRepairParser`, `JsonRepairParser::parse`, `parse_value / parse_object / parse_array / parse_string` [SOURCE_CODE]
- `schema_coercion.rs` - Coerce JSON values to match JSON Schema types (used to repair/convert LLM outputs). Key: `coerce_to_schema`, `coerce_value_with_schema`, `try_coerce_string`, `try_parse_json_string`, `extract_array_from_string` [SOURCE_CODE]

## crates/forge_main/

- `build.rs` - Build script setting package version and name from env [BUILD]

## crates/forge_main/src/

- `banner.rs` - Terminal banner renderer with version and command tips. Key: `DisplayBox`, `display`, `display_zsh_encouragement` [SOURCE_CODE]
- `cli.rs` - Clap-based CLI definitions and top-level command/subcommand structure for the forge binary. Key: `Cli`, `Cli::is_interactive`, `TopLevelCommand`, `Scope`, `Transport` [CLI]
- `conversation_selector.rs` - TUI helper to list and pick a conversation. Key: `ConversationSelector`, `ConversationSelector::select_conversation`, `ConversationRow` [SOURCE_CODE]
- `display_constants.rs` - Centralized display constants and CommandType enum. Key: `status::YES / status::NO`, `markers::EMPTY / markers::BUILT_IN`, `CommandType`, `impl Display for CommandType` [SOURCE_CODE]
- `editor.rs` - Interactive line editor using reedline with completions and history. Key: `ForgeEditor`, `ForgeEditor::new`, `ForgeEditor::prompt`, `ReadResult`, `From<Signal> for ReadResult` [SOURCE_CODE]
- `info.rs` - Terminal information display builder that formats Environment, Config, Metrics, Usage, and Conversation data into aligned sections for CLI output. Key: `Section`, `Section::key`, `Info`, `Info::add_title`, `Info::add_key_value` [SOURCE_CODE]
- `input.rs` - Console wrapper that reads user input via ForgeEditor and parses commands. Key: `Console`, `Console::new`, `Console::prompt`, `Console::set_buffer` [SOURCE_CODE]
- `lib.rs` - Top-level forge_main crate module and public exports. Key: `TRACKER`, `pub use cli::{Cli, TopLevelCommand}` [SOURCE_CODE]
- `main.rs` - CLI/TUI entrypoint that initializes UI and runtime. Key: `main`, `run`, `enable_stdout_vt_processing`, `Cli parsing and piped input detection` [CLI]
- `model.rs` - Manages slash/forge commands, registers agent/workflow commands, and parses input. Key: `ForgeCommandManager`, `ForgeCommandManager::register_agent_commands`, `ForgeCommandManager::parse`, `SlashCommand`, `ForgeCommand` [SOURCE_CODE]
- `oauth_callback.rs` - Localhost OAuth callback HTTP server for capturing authorization codes. Key: `LocalhostOAuthCallbackServer`, `LocalhostOAuthCallbackServer::start`, `LocalhostOAuthCallbackServer::wait_for_code`, `wait_for_localhost_oauth_callback`, `parse_oauth_callback_target` [SOURCE_CODE]
- `porcelain.rs` - Convert Info into a tabular, machine-friendly Porcelain representation. Key: `Porcelain`, `Porcelain::from(&Info)`, `Porcelain::sort_by`, `Porcelain::into_long`, `impl Display for Porcelain` [SOURCE_CODE]
- `prompt.rs` - Specialized reedline Prompt implementation rendering agent, cwd, model, and usage. Key: `ForgePrompt`, `ForgePrompt::render_prompt_left`, `ForgePrompt::render_prompt_right`, `get_git_branch` [SOURCE_CODE]
- `sandbox.rs` - Git worktree sandbox creation helper. Key: `Sandbox`, `new`, `create` [SOURCE_CODE]
- `state.rs` - UI state container for CLI/TUI. Key: `UIState`, `new` [SOURCE_CODE]
- `stream_renderer.rs` - Streaming markdown renderer that coordinates terminal spinner and output. Key: `SharedSpinner`, `StreamingWriter`, `StreamDirectWriter`, `Style` [SOURCE_CODE]
- `sync_display.rs` - Human-friendly display messages for sync progress events. Key: `SyncProgressDisplay`, `impl SyncProgressDisplay for SyncProgress`, `pluralize` [SOURCE_CODE]
- `title_display.rs` - Formats TitleFormat into colored or plain display strings. Key: `TitleDisplay`, `TitleDisplay::with_colors`, `TitleDisplayExt` [SOURCE_CODE]
- `tools_display.rs` - Formats a ToolsOverview into an Info display organized by categories. Key: `format_tools` [SOURCE_CODE]
- `tracker.rs` - Async telemetry event dispatch helpers. Key: `dispatch`, `dispatch_blocking`, `error`, `tool_call`, `set_model` [SOURCE_CODE]
- `ui.rs` - Terminal UI orchestration: interactive loop, subcommand handling, prompts, and display wiring.. Key: `format_mcp_server`, `format_mcp_headers`, `UI`, `init`, `run` [SOURCE_CODE]
- `update.rs` - Check for Forge updates and optionally execute the official updater. Key: `execute_update_command`, `confirm_update`, `on_update` [SOURCE_CODE]
- `utils.rs` - Small presentation utilities for human-friendly time and numbers. Key: `humanize_time`, `humanize_number`, `tests` [SOURCE_CODE]
- `vscode.rs` - VS Code terminal detection and extension installer. Key: `is_vscode_terminal`, `is_extension_installed`, `install_extension`, `should_install_extension` [SOURCE_CODE]

## crates/forge_main/src/completer/

- `command.rs` - Command completer for interactive shell input. Key: `CommandCompleter`, `CommandCompleter::complete` [SOURCE_CODE]
- `input_completer.rs` - Provides fuzzy file and command completion for the interactive input. Key: `InputCompleter`, `complete`, `escape_for_pattern_parse` [SOURCE_CODE]

## crates/forge_main/src/zsh/

- `mod.rs` - Zsh integration helpers and script normalization utility. Key: `normalize_script`, `generate_zsh_plugin / generate_zsh_theme / run_zsh_doctor / run_zsh_keyboard / setup_zsh_integration`, `ZshRPrompt` [SOURCE_CODE]
- `plugin.rs` - Generates zsh plugin/theme content and installs plugin into .zshrc. Key: `ZSH_PLUGIN_LIB`, `generate_zsh_plugin`, `generate_zsh_theme`, `execute_zsh_script_with_streaming`, `setup_zsh_integration` [SOURCE_CODE]
- `rprompt.rs` - Renderer for the ZSH right prompt showing agent, model, tokens, and cost. Key: `ZshRPrompt`, `Display for ZshRPrompt`, `AGENT_SYMBOL`, `MODEL_SYMBOL` [SOURCE_CODE]
- `style.rs` - Helpers to format ZSH prompt escapes with colors and bolding. Key: `ZshColor`, `ZshStyled`, `ZshStyle` [SOURCE_CODE]

## crates/forge_markdown_stream/src/

- `code.rs` - Code block highlighter and line-wrapping renderer using syntect. Key: `CodeHighlighter`, `CodeHighlighter::highlight_line`, `CodeHighlighter::render_code_line` [SOURCE_CODE]
- `heading.rs` - Render markdown headings with theme-aware styling and wrapping. Key: `render_heading`, `HeadingStyler`, `InlineStyler` [SOURCE_CODE]
- `inline.rs` - Renders inline markdown elements to styled strings via a styler. Key: `render_inline_content`, `render_inline_elements` [SOURCE_CODE]
- `lib.rs` - Streaming markdown renderer wrapper (StreamdownRenderer) for terminal output. Key: `StreamdownRenderer`, `StreamdownRenderer::new`, `StreamdownRenderer::push`, `StreamdownRenderer::finish` [SOURCE_CODE]
- `list.rs` - Render nested markdown lists with bullets, numbering, checkboxes and wrapping. Key: `ListState`, `render_list_item`, `strip_checkbox_prefix`, `BULLETS_DASH` [SOURCE_CODE]
- `renderer.rs` - Core event-driven markdown renderer mapping parse events to terminal output. Key: `Renderer`, `Renderer::render_event`, `Renderer::left_margin`, `Renderer::flush_table` [SOURCE_CODE]
- `repair.rs` - Repairs malformed markdown lines (embedded closing fences) before parsing. Key: `repair_line`, `split_embedded_fence` [SOURCE_CODE]
- `style.rs` - Styler traits defining API for markdown element formatting. Key: `InlineStyler`, `HeadingStyler`, `ListStyler`, `TableStyler` [SOURCE_CODE]
- `table.rs` - Table renderer that computes column widths, wraps cells and preserves ANSI. Key: `render_table`, `wrap`, `split_word_at_width` [SOURCE_CODE]
- `theme.rs` - Terminal theme and concrete styler implementations for markdown output. Key: `Style`, `Theme`, `TagStyler`, `Theme::detect` [SOURCE_CODE]
- `utils.rs` - Terminal theme detection utilities (dark or light). Key: `ThemeMode`, `detect_theme_mode` [SOURCE_CODE]

## crates/forge_repo/

- `build.rs` - Protobuf compilation build script [BUILD]

## crates/forge_repo/src/

- `agent.rs` - Load and parse agent definition files from built-in and custom directories. Key: `ForgeAgentRepository`, `load_agents`, `parse_agent_file`, `resolve_agent_conflicts` [SOURCE_CODE]
- `agent_definition.rs` - Agent config deserialization, validation, and conversion to domain Agent. Key: `AgentDefinition`, `AgentDefinition::into_agent`, `tests::test_temperature_validation`, `tests::test_top_p_validation`, `tests::test_top_k_validation` [SOURCE_CODE]
- `context_engine.rs` - gRPC-backed WorkspaceIndexRepository implementation that maps proto RPCs to domain types.. Key: `ForgeContextEngineRepository`, `new`, `with_auth`, `authenticate`, `create_workspace` [SOURCE_CODE]
- `forge_repo.rs` - Repository façade that aggregates infra and persistence implementations and delegates domain repository traits.. Key: `ForgeRepo`, `new`, `SnapshotRepository::insert_snapshot`, `ConversationRepository::upsert_conversation`, `KVStore::cache_get` [SOURCE_CODE]
- `fs_snap.rs` - Filesystem-backed snapshot repository adapter. Key: `ForgeFileSnapshotService`, `new`, `insert_snapshot`, `undo_snapshot` [SOURCE_CODE]
- `fuzzy_search.rs` - gRPC-backed implementation of a fuzzy search repository. Key: `ForgeFuzzySearchRepository`, `ForgeFuzzySearchRepository::new`, `FuzzySearchRepository::fuzzy_search` [SOURCE_CODE]
- `lib.rs` - Forge repository crate root exposing repo modules and proto bindings. Key: `proto_generated`, `pub use forge_repo::*` [SOURCE_CODE]
- `skill.rs` - Skill repository loading built-in, global and project skills. Key: `ForgeSkillRepository`, `ForgeSkillRepository::load_builtin_skills`, `ForgeSkillRepository::load_skills_from_dir`, `extract_skill`, `resolve_skill_conflicts` [SOURCE_CODE]
- `validation.rs` - gRPC-backed implementation of file syntax validation. Key: `ForgeValidationRepository`, `validate_file` [SOURCE_CODE]

## crates/forge_repo/src/conversation/

- `conversation_record.rs` - Repository DTOs mapping conversation domain types to storage records. Key: `ModelIdRecord / ImageRecord / ToolCallIdRecord`, `ToolCallArgumentsRecord`, `TokenCountRecord / UsageRecord`, `ToolValueRecord`, `TextMessageRecord / ToolResultRecord / ToolOutputRecord` [SOURCE_CODE]
- `conversation_repo.rs` - Diesel-backed repository implementation for persisting conversations. Key: `ConversationRepositoryImpl`, `upsert_conversation`, `get_all_conversations`, `delete_conversation` [SOURCE_CODE]

## crates/forge_repo/src/database/

- `pool.rs` - SQLite connection pool builder and migration runner with retry. Key: `PoolConfig`, `DatabasePool`, `SqliteCustomizer`, `MIGRATIONS` [SOURCE_CODE]
- `schema.rs` - Diesel-generated database schema for conversations table [GENERATED]

## crates/forge_repo/src/provider/

- `anthropic.rs` - Anthropic provider client and ChatRepository implementation with streaming SSE support and model discovery.. Key: `Anthropic`, `new`, `get_headers`, `should_use_raw_sse`, `chat` [SOURCE_CODE]
- `bedrock_cache.rs` - Adds Bedrock cache point blocks to converse stream requests. Key: `SetCache`, `transform` [SOURCE_CODE]
- `bedrock_sanitize_ids.rs` - Sanitizes tool call IDs to be compatible with AWS Bedrock constraints. Key: `SanitizeToolIds`, `INVALID_CHARS` [SOURCE_CODE]
- `chat.rs` - Routes chat and model requests to provider-specific repositories and caches models. Key: `ForgeChatRepository`, `ProviderRouter`, `ForgeChatRepository::models`, `ProviderRouter::chat`, `BgRefresh` [SOURCE_CODE]
- `event.rs` - Converts event-source SSE stream to chat completion message stream. Key: `into_chat_completion_message` [SOURCE_CODE]
- `google.rs` - Google provider client and repository for streaming chat and model listing. Key: `Google<T>`, `Google::chat`, `Google::models`, `GoogleResponseRepository<F>`, `GoogleResponseRepository::create_client` [SOURCE_CODE]
- `mock_server.rs` - HTTP mock server helpers for provider unit tests. Key: `MockServer`, `MockServer::mock_models`, `MockServer::mock_responses_stream`, `normalize_ports` [SOURCE_CODE]
- `mod.rs` - Provider module aggregator and conversion traits for provider types. Key: `pub use chat::*`, `IntoDomain`, `FromDomain` [SOURCE_CODE]
- `openai.rs` - OpenAI-compatible provider client handling chat streaming and model listing. Key: `OpenAIProvider`, `OpenAIProvider::get_headers_with_request`, `OpenAIProvider::inner_chat`, `OpenAIResponseRepository`, `enhance_error` [SOURCE_CODE]
- `opencode_zen.rs` - Routes OpenCode Zen calls to appropriate backend per model prefix. Key: `OpenCodeZenResponseRepository`, `get_backend`, `build_provider`, `OpenCodeBackend` [SOURCE_CODE]
- `provider_repo.rs` - Provider registry and credential migration logic for model providers. Key: `ProviderConfig`, `UrlParamVarConfig`, `ProviderConfigs`, `ForgeProviderRepository`, `migrate_env_to_file` [SOURCE_CODE]
- `retry.rs` - Classifies errors to decide whether they are retryable based on heuristics. Key: `into_retry`, `is_api_transport_error`, `get_api_status_code`, `is_anthropic_overloaded_error` [SOURCE_CODE]
- `utils.rs` - HTTP helper utilities for provider requests. Key: `format_http_context`, `join_url`, `create_headers` [SOURCE_CODE]

## crates/forge_repo/src/provider/openai_responses/

- `request.rs` - Converts domain chat context to OpenAI Responses API requests. Key: `map_reasoning_details_to_input_items`, `FromDomain<ReasoningConfig> for oai::Reasoning`, `codex_tool_parameters`, `FromDomain<ChatContext> for oai::CreateResponse`, `FromDomain<ToolChoice> for oai::ToolChoiceParam` [SOURCE_CODE]

## crates/forge_select/src/

- `confirm.rs` - Builder and prompt logic for yes/no confirmation prompts. Key: `ConfirmBuilder`, `ConfirmBuilder::with_default`, `ConfirmBuilder::prompt` [SOURCE_CODE]
- `input.rs` - Interactive single-line input builder using rustyline with paste handling. Key: `InputBuilder`, `InputBuilder::prompt`, `strip_bracketed_paste` [SOURCE_CODE]
- `lib.rs` - Public re-exports for selection/input widgets. Key: `ForgeWidget`, `SelectBuilder`, `MultiSelectBuilder`, `InputBuilder` [SOURCE_CODE]
- `multi.rs` - Multi-select prompt backed by fzf with ANSI stripping and result parsing. Key: `MultiSelectBuilder`, `MultiSelectBuilder::prompt`, `build_multi_fzf` [SOURCE_CODE]
- `select.rs` - Interactive fuzzy select builder using fzf. Key: `SelectBuilder`, `build_fzf`, `indexed_items`, `parse_fzf_index`, `SelectBuilder::prompt` [SOURCE_CODE]
- `widget.rs` - Factory for selection and prompt builders (fzf-based). Key: `ForgeWidget`, `ForgeWidget::select`, `ForgeWidget::confirm`, `ForgeWidget::input`, `ForgeWidget::multi_select` [SOURCE_CODE]

## crates/forge_services/src/

- `agent_registry.rs` - In-memory agent registry with lazy loading and active-agent management. Key: `ForgeAgentRegistryService`, `ensure_agents_loaded`, `load_agents`, `impl forge_app::AgentRegistry for ForgeAgentRegistryService` [SOURCE_CODE]
- `app_config.rs` - Infra-backed application config service that reads/updates provider & model defaults.. Key: `ForgeAppConfigService`, `get_default_provider`, `get_provider_model`, `get_commit_config`, `get_suggest_config` [SOURCE_CODE]
- `attachment.rs` - Produces Attachment values (file contents or directory listings) from file tags/URLs using infra traits.. Key: `ForgeChatRequest`, `prepare_attachments`, `populate_attachments`, `attachments` [SOURCE_CODE]
- `auth.rs` - Auth service that fetches user info and usage from services API. Key: `ForgeAuthService`, `ForgeAuthService::user_info`, `ForgeAuthService::user_usage` [SOURCE_CODE]
- `clipper.rs` - Text clipping/truncation strategies and helpers. Key: `ClipperResult`, `Clipper`, `clip`, `MAX_LIMIT` [SOURCE_CODE]
- `command.rs` - Load and parse built-in and custom command definitions. Key: `CommandLoaderService`, `parse_command_file`, `resolve_command_conflicts` [SOURCE_CODE]
- `context_engine.rs` - Workspace indexing/search service bridge that orchestrates sync, search, and workspace lifecycle.. Key: `ForgeWorkspaceService`, `sync_codebase_internal`, `get_workspace_credentials`, `find_workspace_by_path`, `get_workspace_by_path` [SOURCE_CODE]
- `conversation.rs` - Service adapter implementing conversation management over a repository. Key: `ForgeConversationService`, `ForgeConversationService::new`, `ConversationService::modify_conversation`, `ConversationService::find_conversation`, `ConversationService::upsert_conversation` [SOURCE_CODE]
- `discovery.rs` - File discovery service adapter for environment and walker infra. Key: `ForgeDiscoveryService`, `discover_with_config`, `list_current_directory` [SOURCE_CODE]
- `error.rs` - Error enum for authentication and provider flows. Key: `Error` [SOURCE_CODE]
- `fd.rs` - Workspace file discovery and filtering utilities with git-fallback. Key: `ALLOWED_EXTENSIONS`, `filter_and_resolve`, `FileDiscovery`, `FdDefault`, `discover_sync_file_paths` [SOURCE_CODE]
- `fd_git.rs` - Git-backed file discovery using `git ls-files`. Key: `FsGit`, `FsGit::new`, `FsGit::git_ls_files`, `FileDiscovery::discover (impl for FsGit)` [SOURCE_CODE]
- `fd_walker.rs` - Filesystem walker fallback for file discovery. Key: `FdWalker`, `FdWalker::new`, `FileDiscovery::discover (impl for FdWalker)` [SOURCE_CODE]
- `forge_services.rs` - Application container that composes and exposes Forge runtime services built on top of an infra implementation. Key: `ForgeServices`, `new`, `McpService`, `AuthService`, `Services for ForgeServices<F>` [SOURCE_CODE]
- `instructions.rs` - Service to discover and read AGENTS.md custom instruction files. Key: `ForgeCustomInstructionsService`, `discover_agents_files`, `get_git_root`, `get_custom_instructions` [SOURCE_CODE]
- `lib.rs` - Service crate exports and conversion traits. Key: `IntoDomain`, `FromDomain`, `mod provider_service` [SOURCE_CODE]
- `metadata.rs` - Simple metadata builder and display formatter. Key: `Metadata`, `add`, `add_optional` [SOURCE_CODE]
- `policy.rs` - Policy management and interactive permission decision service. Key: `ForgePolicyService`, `PolicyPermission`, `DEFAULT_POLICIES`, `create_policy_for_operation` [SOURCE_CODE]
- `provider_auth.rs` - Provider authentication flows and credential refresh logic. Key: `ForgeProviderAuthService`, `init_provider_auth`, `complete_provider_auth`, `refresh_provider_credential` [SOURCE_CODE]
- `provider_service.rs` - Service wrapper that renders provider templates and delegates provider/model calls. Key: `ForgeProviderService`, `render_url_template`, `render_provider`, `ProviderService impl for ForgeProviderService` [SOURCE_CODE]
- `range.rs` - Line-range resolution and validation utility. Key: `resolve_range` [SOURCE_CODE]
- `sync.rs` - Engine to sync workspace files with remote index (hash/compare/upload/delete). Key: `FileReadError`, `canonicalize_path`, `extract_failed_statuses`, `WorkspaceSyncEngine`, `WorkspaceSyncEngine::run` [SOURCE_CODE]
- `template.rs` - Template registration and rendering service using Handlebars. Key: `ForgeTemplateService`, `get_hb`, `read_all`, `compile_template` [SOURCE_CODE]

## crates/forge_services/src/tool_services/

- `fs_patch.rs` - File patching service with snapshot coordination and fuzzy fallback. Key: `Range`, `Error`, `compute_range`, `apply_replacement`, `ForgeFsPatch` [SOURCE_CODE]
- `fs_read.rs` - Filesystem read tool service with MIME detection, size checks, and line truncation. Key: `ForgeFsRead`, `assert_file_size`, `detect_mime_type`, `truncate_line` [SOURCE_CODE]
- `fs_search.rs` - Filesystem search service using grep crates with advanced options. Key: `ForgeFsSearch<W>`, `ForgeFsSearch::search`, `ContextSink`, `build_matcher / get_matching_files / search_files_with_matches / search_count / search_content` [SOURCE_CODE]
- `fs_write.rs` - File write service with snapshot coordination and validation. Key: `ForgeFsWrite`, `FsWriteService::write`, `compute_hash` [SOURCE_CODE]
- `image_read.rs` - Service to read and validate image files into Image domain objects. Key: `ForgeImageRead`, `ImageFormat`, `read_image` [SOURCE_CODE]
- `mod.rs` - Re-exports the tool service modules for the crate. Key: `fetch`, `fs_read`, `fs_write`, `shell`, `skill` [SOURCE_CODE]
- `shell.rs` - Shell command execution wrapper that validates and strips ANSI. Key: `ForgeShell`, `strip_ansi`, `validate_command`, `execute` [SOURCE_CODE]

## crates/forge_snaps/src/

- `lib.rs` - Snapshot crate public re-exports. Key: `service`, `pub use service::*` [SOURCE_CODE]
- `service.rs` - Filesystem snapshot service to create and undo file snapshots. Key: `SnapshotService`, `SnapshotService::new`, `SnapshotService::create_snapshot`, `SnapshotService::find_recent_snapshot`, `SnapshotService::undo_snapshot` [SOURCE_CODE]

## crates/forge_spinner/src/

- `lib.rs` - Spinner and progress-bar manager that handles terminal spinner lifecycle and elapsed-time formatting.. Key: `TICK_DURATION_MS`, `TICKS`, `format_elapsed_time`, `SpinnerManager`, `SpinnerManager::start` [SOURCE_CODE]
- `progress_bar.rs` - Manage determinate progress bar using indicatif. Key: `ProgressBarManager`, `ProgressBarManager::start`, `ProgressBarManager::stop` [SOURCE_CODE]

## crates/forge_stream/src/

- `mpsc_stream.rs` - Wraps a tokio mpsc producer as a futures::Stream and aborts task on drop. Key: `MpscStream`, `MpscStream::spawn`, `Stream for MpscStream::poll_next`, `Drop for MpscStream`, `test_stream_receives_messages` [SOURCE_CODE]

## crates/forge_template/src/

- `element.rs` - Lightweight HTML/XML element builder with rendering and escaping. Key: `Element`, `Element::new`, `Element::text`, `Element::cdata`, `Element::attr / class / append / render` [SOURCE_CODE]

## crates/forge_tool_macros/src/

- `lib.rs` - Proc-macros to derive ToolDescription from docs or external file. Key: `tool_description_file`, `derive_description` [SOURCE_CODE]

## crates/forge_tracker/src/

- `can_track.rs` - Checks whether telemetry/tracking should be enabled based on version. Key: `VERSION`, `can_track`, `can_track_inner` [SOURCE_CODE]
- `dispatch.rs` - Telemetry tracker that rate-limits and dispatches events to collectors. Key: `Tracker`, `tracking_enabled`, `system_info`, `dispatch` [SOURCE_CODE]
- `error.rs` - Unified error enum and result alias for tracker operations. Key: `Error`, `Result` [SOURCE_CODE]
- `event.rs` - Telemetry event types and payload serialization helpers. Key: `Event`, `Name`, `ToolCallPayload`, `EventKind`, `Identity` [SOURCE_CODE]
- `lib.rs` - Tracker crate public exports and module wiring. Key: `VERSION`, `Tracker`, `Result`, `Event`, `Guard` [SOURCE_CODE]
- `log.rs` - Tracing/logging initialization with optional PostHog writer. Key: `init_tracing`, `prepare_writer`, `Guard`, `PostHogWriter` [SOURCE_CODE]
- `rate_limit.rs` - Simple fixed-window rate limiter for events. Key: `RateLimiter`, `RateLimiter::inc_and_check`, `RateLimiter::check_at` [SOURCE_CODE]

## crates/forge_walker/src/

- `walker.rs` - Filesystem walker that enumerates files/dirs with limits and binary filtering. Key: `File`, `Walker`, `Walker::get`, `Walker::get_blocking`, `Walker::is_likely_binary` [SOURCE_CODE]

## scripts/

- `benchmark.sh` - Shell script that builds the workspace, runs a forge command multiple times and reports timing statistics with an optional threshold check. Key: `BASE_COMMAND`, `THRESHOLD`, `ARGS`, `COMMAND`, `ITERATIONS` [BUILD]
- `list-all-porcelain.sh` - Runs all `forge list ... --porcelain` commands and prints results with timing. Key: `FORGE_BIN`, `print_section`, `print_command`, `print_runtime` [SOURCE_CODE]

## shell-plugin/

- `doctor.zsh` - Zsh environment diagnostic tool for Forge shell integration. Key: `print_section`, `print_result`, `version_gte`, `plugins, _FORGE_PLUGIN_LOADED, _FORGE_THEME_LOADED` [SOURCE_CODE]
- `forge.plugin.zsh` - Main zsh plugin loader that sources modular plugin components. Key: `source "${0:A:h}/lib/config.zsh"`, `source "${0:A:h}/lib/dispatcher.zsh"`, `source "${0:A:h}/lib/bindings.zsh"` [SOURCE_CODE]
- `forge.setup.zsh` - Managed .zshrc block inserted by `forge zsh setup` [CONFIG]
- `forge.theme.zsh` - Zsh theme snippet that sets RPROMPT using Forge CLI output. Key: `_forge_prompt_info`, `RPROMPT` [SOURCE_CODE]
- `keyboard.zsh` - Displays platform- and mode-specific ZLE keyboard shortcuts. Key: `print_shortcut`, `print_section`, `platform detection` [SOURCE_CODE]

## shell-plugin/lib/

- `bindings.zsh` - ZLE widget registrations and key bindings for the forge plugin. Key: `forge-bracketed-paste`, `forge-accept-line`, `forge-completion` [SOURCE_CODE]
- `completion.zsh` - Custom tab-completion widget handling :commands and @ file picks. Key: `forge-completion` [SOURCE_CODE]
- `config.zsh` - Configuration variables and environment detection for the shell plugin [CONFIG]
- `dispatcher.zsh` - Main dispatcher for :commands and the accept-line widget logic. Key: `_forge_action_default`, `forge-accept-line` [SOURCE_CODE]
- `helpers.zsh` - Utility helpers for executing forge CLI calls and UI helpers. Key: `_forge_get_commands`, `_forge_exec_interactive`, `_forge_exec`, `_forge_log`, `_forge_start_background_sync` [SOURCE_CODE]
- `highlight.zsh` - Syntax highlighting patterns for forge conversation syntax [CONFIG]

## shell-plugin/lib/actions/

- `auth.zsh` - ZSH actions to login and logout providers via fzf selection. Key: `_forge_action_login`, `_forge_action_logout` [SOURCE_CODE]
- `config.zsh` - Config and model/provider/agent selection actions with fzf. Key: `_forge_action_agent`, `_forge_pick_model`, `_forge_action_model`, `_forge_action_session_model`, `_forge_action_config_edit` [SOURCE_CODE]
- `conversation.zsh` - Conversation management actions: list, switch, clone, copy, and rename. Key: `_forge_switch_conversation`, `_forge_clear_conversation`, `_forge_action_conversation`, `_forge_action_clone`, `_forge_action_copy` [SOURCE_CODE]
- `core.zsh` - Core conversation and session action handlers. Key: `_forge_action_new`, `_forge_action_info`, `_forge_handle_conversation_command`, `_forge_action_dump` [SOURCE_CODE]
- `doctor.zsh` - Run environment diagnostics via the forge doctor command. Key: `_forge_action_doctor` [SOURCE_CODE]
- `editor.zsh` - Open external editor and generate shell commands from descriptions. Key: `_forge_action_editor`, `_forge_action_suggest` [SOURCE_CODE]
- `git.zsh` - Git commit helpers that generate AI commit messages. Key: `_forge_action_commit`, `_forge_action_commit_preview` [SOURCE_CODE]
- `keyboard.zsh` - Display keyboard shortcuts via the forge keyboard command. Key: `_forge_action_keyboard` [SOURCE_CODE]
- `provider.zsh` - Provider fzf selection helper for the shell plugin. Key: `_forge_select_provider` [SOURCE_CODE]


---
*This knowledge base was extracted by [Codeset](https://codeset.ai) and is available via `python .codex/docs/get_context.py <file_or_folder>`*
