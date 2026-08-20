# Forgecode Compaction System Enhancement Plan

## Objective

Enhance the forgecode context compaction system from a purely structural extraction approach to a hybrid system that combines **intelligent pre-processing**, **LLM-based semantic summarization**, and **adaptive eviction strategies** to maximize context retention of meaningful information while maintaining deterministic performance.

---

## SOTA Research Summary

### Current Industry Approaches

| Approach | Provider | Characteristics |
|----------|----------|----------------|
| **Structural Extraction** | Current forgecode | Fast, deterministic, low semantic fidelity |
| **LLM Summarization** | Claude Code, OpenAI Agents | High fidelity, slow (~500ms+), expensive |
| **Hybrid Extraction** | Microsoft Copilot | Combines extraction + LLM refinement |
| **Importance Scoring** | Cursor AI | Scores messages by relevance, preserves high-value |
| **Incremental Summarization** | Perplexity AI | Accumulates summaries, reduces redundancy |
| **Semantic Chunking** | LangChain | Groups semantically similar content |

### Key Findings from Anthropic Documentation

1. **Compaction timing is critical**: Trigger at 70-80% of context window to preserve headroom
2. **Tool call atomicity**: Never split tool calls from their results
3. **Extended thinking preservation**: Reasoning chains must be maintained for model continuity
4. **Summary quality matters**: Poor summaries degrade subsequent model performance

### Best Practices Identified

1. **Pre-compaction filtering**: Remove noise before summarization
2. **Adaptive eviction windows**: More aggressive near context limits
3. **Importance-based preservation**: High-value messages protected from eviction
4. **Structured summaries**: Machine-parseable formats improve downstream processing
5. **Cost-latency tradeoff**: Cheaper models can be used for summarization

---

## Implementation Plan

### Phase 1 — Enhanced Configuration (`forge_config` + `forge_domain`)

#### Task 1: Extend `CompactConfig` with new options

**Files:** `crates/forge_config/src/compact.rs`, `crates/forge_domain/src/compact/compact_config.rs`

```rust
// New fields in CompactConfig
pub struct Compact {
    // ... existing fields ...

    /// Strategy for summarization: extract only, llm, or hybrid
    #[serde(default)]
    pub summarization_strategy: SummarizationStrategy,

    /// Enable pre-compaction filtering
    #[serde(default)]
    pub enable_prefilter: bool,

    /// Enable adaptive eviction window
    #[serde(default)]
    pub enable_adaptive_eviction: bool,

    /// Enable importance-based preservation
    #[serde(default)]
    pub enable_importance_scoring: bool,

    /// Maximum tokens in generated summary
    #[serde(default)]
    pub summary_max_tokens: Option<usize>,
}

pub enum SummarizationStrategy {
    /// Pure structural extraction (current behavior)
    Extract,
    /// LLM-based semantic summarization
    Llm,
    /// Hybrid: extract then refine with LLM
    Hybrid,
}
```

#### Task 2: Add `CompactionHistory` for incremental tracking

**Files:** `crates/forge_domain/src/compact/history.rs`, `crates/forge_domain/src/compact/mod.rs`

```rust
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct CompactionHistory {
    /// Content hashes of past summaries to detect redundancy
    pub summary_hashes: Vec<u64>,
    /// Last seen file versions (path -> hash)
    pub file_versions: HashMap<PathBuf, String>,
    /// Count of successful compactions
    pub compaction_count: usize,
    /// Total tokens reduced across all compactions
    pub total_tokens_reduced: usize,
}
```

#### Task 3: Add `ImportanceScore` to messages

**Files:** `crates/forge_domain/src/context.rs`

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageImportance {
    /// Base importance score (0-100)
    pub score: u8,
    /// Factors contributing to score
    pub factors: Vec<ImportanceFactor>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ImportanceFactor {
    HasToolCalls,
    HasErrors,
    HasFileChanges,
    HasUserIntent,
    ReasoningChain,
    Decision,
}
```

---

### Phase 2 — Enhanced Eviction Strategy (`forge_domain`)

#### Task 4: Implement adaptive eviction window

**Files:** `crates/forge_domain/src/compact/strategy.rs`

```rust
impl CompactionStrategy {
    /// Calculate adaptive eviction percentage based on context state
    pub fn adaptive_eviction(&self, context: &Context, threshold: usize) -> f64 {
        let token_count = context.token_count();
        let ratio = token_count as f64 / threshold as f64;

        // Eviction aggressiveness increases as we approach threshold
        match ratio {
            r if r > 0.95 => 0.5,  // 50% - critical zone
            r if r > 0.85 => 0.35, // 35% - warning zone
            r if r > 0.70 => 0.2, // 20% - normal
            _ => 0.1,              // 10% - conservative
        }
    }
}
```

#### Task 5: Implement importance-based message scoring

**Files:** `crates/forge_domain/src/compact/importance.rs`

```rust
impl MessageImportance {
    pub fn calculate(msg: &ContextMessage) -> Self {
        let mut score: u8 = 50; // Base score
        let mut factors = Vec::new();

        match msg.deref() {
            ContextMessage::Text(t) => {
                if t.tool_calls.is_some() {
                    score += 20;
                    factors.push(ImportanceFactor::HasToolCalls);
                }
                if t.reasoning_details.is_some() {
                    score += 15;
                    factors.push(ImportanceFactor::ReasoningChain);
                }
            }
            ContextMessage::Tool(r) if r.is_error() => {
                score = 100; // Critical
                factors.push(ImportanceFactor::HasErrors);
            }
            _ => {}
        }

        Self { score, factors }
    }

    /// Minimum importance required to survive compaction
    pub const MIN_SURVIVAL_SCORE: u8 = 60;
}
```

#### Task 6: Enhanced eviction range finding with importance

**Files:** `crates/forge_domain/src/compact/strategy.rs`

```rust
fn find_eviction_range_with_importance(
    context: &Context,
    max_retention: usize,
    history: &CompactionHistory,
) -> Option<(usize, usize)> {
    let messages = &context.messages;

    // Filter out high-importance messages from eviction candidates
    let eviction_candidates: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, msg)| {
            let importance = MessageImportance::calculate(msg);
            importance.score < MessageImportance::MIN_SURVIVAL_SCORE
        })
        .map(|(i, _)| i)
        .collect();

    // Find range using only eviction candidates
    find_sequence_preserving_last_n(context, max_retention)
        .map(|(start, end)| {
            // Adjust range to exclude protected messages
            let protected: Vec<usize> = messages
                .iter()
                .enumerate()
                .filter(|(_, msg)| {
                    let importance = MessageImportance::calculate(msg);
                    importance.score >= MessageImportance::MIN_SURVIVAL_SCORE
                })
                .map(|(i, _)| i)
                .collect();

            // If protected messages fall in eviction range, shrink it
            let new_start = protected.iter().find(|&&i| i >= start).copied().unwrap_or(start);
            (new_start.max(start), end)
        })
}
```

---

### Phase 3 — LLM Summarization (`forge_app`)

#### Task 7: Create summarization prompt template

**Files:** `templates/forge-summarization-prompt.md` (new)

```markdown
You are a precise code assistant summarizing previous conversation context.

## Task
Summarize the following conversation history into a concise, structured format that preserves:
1. Key decisions and their rationale
2. Files modified and their purposes
3. Tool operations performed and their outcomes
4. Important constraints or requirements discovered

## Format
Provide a summary with these sections:

### Decisions
- [List key architectural/implementation decisions]

### Files Changed
- `path/to/file`: Brief description of changes

### Operations Summary
- **Read**: [files read and why]
- **Write/Modify**: [files changed and what]
- **Execute**: [commands run and outcomes]
- **Search**: [patterns searched and findings]

### Discovered Constraints
- [Any limitations, requirements, or context important for continuation]

### Current State
- [Where work left off, what's next]

## Conversation to Summarize
{{conversation}}
```

#### Task 8: Implement `LlmSummarizer` service

**Files:** `crates/forge_app/src/services/summarizer.rs`, `crates/forge_app/src/lib.rs`

```rust
pub struct LlmSummarizer {
    provider: Arc<dyn Provider>,
    template_engine: TemplateEngine,
    compact_config: Compact,
}

impl LlmSummarizer {
    pub async fn summarize(
        &self,
        context: &Context,
        history: &CompactionHistory,
    ) -> anyhow::Result<String> {
        // Render summarization prompt
        let prompt = self.template_engine.render(
            "forge-summarization-prompt.md",
            &serde_json::json!({
                "conversation": self.extract_conversation_text(context),
                "history_summary": self.summarize_history(history),
            }),
        )?;

        // Create summary context
        let summary_context = Context::default()
            .add_message(ContextMessage::user(prompt, None));

        // Use compact model if configured, otherwise agent model
        let model = self.compact_config.model.as_ref()
            .cloned()
            .unwrap_or_else(|| ModelId::new("claude-3-5-haiku"));

        // Generate summary
        let response = self.provider.chat(&model, summary_context).await?;
        self.collect_content(response).await
    }

    fn extract_conversation_text(&self, context: &Context) -> String {
        // Convert context to readable text format
        context.messages.iter()
            .map(|msg| format_message(msg))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}
```

#### Task 9: Integrate summarization into Compactor

**Files:** `crates/forge_app/src/compact.rs`

```rust
impl Compactor {
    pub fn compact(&self, context: Context, max: bool) -> anyhow::Result<Context> {
        let strategy = self.build_strategy(&context, max);

        match strategy.eviction_range(&context) {
            Some(sequence) => {
                match self.compact.summarization_strategy {
                    SummarizationStrategy::Extract => {
                        self.compress_single_sequence(context, sequence)
                    }
                    SummarizationStrategy::Llm => {
                        self.compress_with_llm(context, sequence).await
                    }
                    SummarizationStrategy::Hybrid => {
                        // Extract first, then refine with LLM
                        let extracted = self.compress_single_sequence(context.clone(), sequence)?;
                        self.refine_summary(&extracted).await
                    }
                }
            }
            None => Ok(context),
        }
    }

    async fn compress_with_llm(
        &self,
        mut context: Context,
        sequence: (usize, usize),
    ) -> anyhow::Result<Context> {
        let (start, end) = sequence;

        // Extract the sequence for summarization
        let sequence_context = context
            .messages
            .get(start..=end)
            .map(|slice| slice.to_vec())
            .unwrap_or_default();

        // Create temporary context for LLM
        let temp_context = Context::default().messages(sequence_context);

        // Get LLM summary
        let llm_summary = self.summarizer.summarize(&temp_context, &self.history).await?;

        // Apply transformers to the extracted summary
        let summary = self.transform(ContextSummary::from(&temp_context));

        // Combine LLM summary with structured summary
        let combined_summary = format!(
            "{}\n\n## Structured Operations\n{}",
            llm_summary,
            self.render_structured_summary(&summary)
        );

        // Replace range with summary
        let summary_entry = MessageEntry::from(ContextMessage::user(combined_summary, None));
        context.messages.splice(start..=end, std::iter::once(summary_entry));

        // Update history
        self.history.record_compaction(&context);

        Ok(context)
    }

    async fn refine_summary(&self, context: &Context) -> anyhow::Result<Context> {
        // Light LLM refinement of already-extracted summary
        // (Implementation details)
        Ok(context.clone())
    }
}
```

---

### Phase 4 — Pre-Compaction Filtering (`forge_app`)

#### Task 10: Implement pre-compaction filters

**Files:** `crates/forge_app/src/transformers/prefilter.rs`

```rust
pub struct PreCompactionFilter {
    /// Minimum length for tool results (shorter = likely empty/error)
    pub min_tool_result_length: usize,
    /// Patterns for debug output to strip
    pub debug_patterns: Vec<Regex>,
}

impl PreCompactionFilter {
    pub fn filter(&self, context: &mut Context) {
        context.messages.retain(|msg| {
            match msg.deref() {
                ContextMessage::Tool(r) => {
                    // Keep tool results above minimum length
                    r.output.text_len() >= self.min_tool_result_length
                }
                ContextMessage::Text(t) => {
                    // Filter out debug output patterns
                    !self.debug_patterns.iter().any(|p| p.is_match(&t.content))
                }
                _ => true
            }
        });
    }

    /// Collapse duplicate consecutive tool calls (same tool, same args)
    pub fn collapse_duplicates(&self, context: &mut Context) {
        let mut deduped = Vec::new();
        let mut prev_call: Option<(String, String)> = None;

        for msg in context.messages.drain(..) {
            if let ContextMessage::Text(t) = msg {
                if let Some(calls) = &t.tool_calls {
                    for call in calls {
                        let key = (call.name.to_string(), call.arguments.to_string());
                        if prev_call.as_ref() != Some(&key) {
                            prev_call = Some(key);
                            deduped.push(ContextMessage::Text(t));
                        }
                    }
                } else {
                    deduped.push(ContextMessage::Text(t));
                }
            } else {
                deduped.push(msg);
            }
        }

        context.messages = deduped;
    }
}
```

---

### Phase 5 — Enhanced Summary Template (`forge_app`)

#### Task 11: Create enhanced summary frame

**Files:** `templates/forge-partial-summary-frame-v2.md`

```markdown
{{#if structured}}
## Prior Context Summary

**Files Modified:**
{{#each files}}
- `{{path}}`: {{description}}
{{/each}}

**Operations:**
- **Reads**: {{read_count}} files
- **Writes/Modifies**: {{write_count}} files
- **Executions**: {{executions}}
- **Searches**: {{searches}}

{{#if decisions}}
**Key Decisions:**
{{#each decisions}}
- {{this}}
{{/each}}
{{/if}}

{{#if constraints}}
**Constraints Discovered:**
{{#each constraints}}
- {{this}}
{{/each}}
{{/if}}

**Progress:** {{completed_tasks}}/{{total_tasks}} tasks completed
{{/if}}

{{#if llm_summary}}
{{llm_summary}}
{{/if}}

---
*This summary was generated from {{compaction_count}} previous compaction(s).*
{{/if}}

Proceed with implementation based on this context.
```

---

### Phase 6 — Metrics & Observability

#### Task 12: Add compaction metrics collection

**Files:** `crates/forge_domain/src/compact/metrics.rs`

```rust
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct CompactionMetrics {
    /// Number of times compaction triggered
    pub compaction_count: usize,
    /// Total tokens reduced
    pub total_tokens_reduced: usize,
    /// Average token reduction per compaction
    pub avg_token_reduction: f64,
    /// Total messages reduced
    pub total_messages_reduced: usize,
    /// Compaction strategies used
    pub strategies_used: HashMap<String, usize>,
    /// Errors encountered
    pub errors: Vec<CompactionError>,
}

impl CompactionMetrics {
    pub fn record(&mut self, result: &CompactionResult, strategy: &str) {
        self.compaction_count += 1;
        self.total_tokens_reduced +=
            result.original_tokens.saturating_sub(result.compacted_tokens);
        self.total_messages_reduced +=
            result.original_messages.saturating_sub(result.compacted_messages);
        *self.strategies_used.entry(strategy.to_string()).or_insert(0) += 1;
    }
}
```

---

## Verification Criteria

1. **Functional correctness:**
   - [ ] Compaction triggers at configured thresholds
   - [ ] Tool calls remain atomic after compaction
   - [ ] Extended thinking reasoning preserved
   - [ ] Usage accumulation works correctly
   - [ ] Droppable messages removed

2. **Enhanced features:**
   - [ ] Adaptive eviction adjusts based on context ratio
   - [ ] Importance scoring protects high-value messages
   - [ ] LLM summarization produces coherent summaries
   - [ ] Pre-filter removes noise before compaction
   - [ ] History tracking prevents redundant summaries

3. **Performance:**
   - [ ] Structural extraction: <5ms
   - [ ] LLM summarization: <2s with timeout
   - [ ] No memory leaks from history accumulation

4. **Backward compatibility:**
   - [ ] Existing `compact` config remains valid
   - [ ] Default behavior unchanged (structural extraction)
   - [ ] Migration path for existing conversations

---

## Potential Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| LLM summarization adds latency | Medium | Use cheaper models (haiku), cache summaries, timeout after 3s |
| Poor LLM summary quality | High | Fallback to structural extraction, validate summary format |
| History accumulation memory growth | Low | Limit history size, compress older entries |
| Importance scoring misclassification | Medium | Allow configuration of thresholds, provide defaults |
| Adaptive eviction too aggressive | Low | Provide conservative defaults, allow tuning |

---

## Alternative Approaches

1. **Pure LLM Approach**: Use LLM for all summarization, skip structural extraction
   - Pros: Higher semantic fidelity
   - Cons: Slower, more expensive, less deterministic

2. **Semantic Embedding Approach**: Use embeddings to find and preserve semantically important messages
   - Pros: Better relevance scoring
   - Cons: Requires embedding service, more complex

3. **Streaming Compaction**: Compact incrementally as context grows, not at threshold
   - Pros: More predictable latency, smoother context growth
   - Cons: More complex state management

4. **Multi-Model Cascade**: Start with extraction, escalate to LLM for complex contexts
   - Pros: Balances cost and quality
   - Cons: Most complex implementation

---

## Phased Rollout

| Phase | Features | Risk Level | Duration |
|-------|----------|------------|----------|
| Phase 1 | Config extensions, adaptive eviction | Low | 1 week |
| Phase 2 | Importance scoring, pre-filtering | Low | 1 week |
| Phase 3 | LLM summarization (opt-in) | Medium | 2 weeks |
| Phase 4 | Metrics, observability | Low | 1 week |
| Phase 5 | Template improvements | Low | 1 week |

---

## References

- Anthropic Context Windows Documentation
- OpenAI Conversation State Management
- Microsoft Copilot Context Management
- LangChain Context Management Strategies
