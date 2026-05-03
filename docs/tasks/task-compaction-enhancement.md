# TASK: Enhanced Compaction System

**ID:** task-compaction-enhancement
**Status:** Open
**Created:** 2026-05-02
**Parent Plan:** `plans/2026-05-02-compaction-enhancement-v1.md`
**Related ADR:** `docs/adr/0001-compaction-summarization-strategy.md`

---

## Objective

Enhance the forgecode context compaction system with LLM-based semantic summarization, adaptive eviction, importance-based preservation, and pre-compaction filtering.

---

## Tasks

### Phase 1 — Configuration & Core Types

- [ ] **T1.1:** Extend `CompactConfig` with new options (`crates/forge_config/src/compact.rs`)
  - Add `summarization_strategy: SummarizationStrategy`
  - Add `enable_prefilter: bool`
  - Add `enable_adaptive_eviction: bool`
  - Add `enable_importance_scoring: bool`
  - Add `summary_max_tokens: Option<usize>`

- [ ] **T1.2:** Create `CompactionHistory` struct (`crates/forge_domain/src/compact/history.rs`)
  - `summary_hashes: Vec<u64>`
  - `file_versions: HashMap<PathBuf, String>`
  - `compaction_count: usize`
  - `total_tokens_reduced: usize`

- [ ] **T1.3:** Create `ImportanceScore` types (`crates/forge_domain/src/compact/importance.rs`)
  - `MessageImportance` struct
  - `ImportanceFactor` enum
  - `calculate()` function
  - `MIN_SURVIVAL_SCORE` constant

### Phase 2 — Eviction Strategy

- [ ] **T2.1:** Implement adaptive eviction window (`crates/forge_domain/src/compact/strategy.rs`)
  - `adaptive_eviction()` function
  - Configurable via `enable_adaptive_eviction`

- [ ] **T2.2:** Implement importance-based range finding
  - Filter protected messages from eviction candidates
  - Preserve high-importance messages

### Phase 3 — LLM Summarization

- [ ] **T3.1:** Create summarization prompt template (`templates/forge-summarization-prompt.md`)
  - Structured prompt for LLM summarization
  - Include conversation context and history

- [ ] **T3.2:** Implement `LlmSummarizer` service (`crates/forge_app/src/services/summarizer.rs`)
  - `summarize()` async function
  - Model selection (compact model or agent model)
  - Timeout handling

- [ ] **T3.3:** Integrate into `Compactor` (`crates/forge_app/src/compact.rs`)
  - Add summarization strategy handling
  - Hybrid mode: extract then refine
  - Fallback on LLM failure

### Phase 4 — Pre-Compaction Filtering

- [ ] **T4.1:** Implement `PreCompactionFilter` (`crates/forge_app/src/transformers/prefilter.rs`)
  - `filter()` function
  - `collapse_duplicates()` function
  - Minimum tool result length
  - Debug pattern removal

### Phase 5 — Templates & Output

- [ ] **T5.1:** Create enhanced summary frame (`templates/forge-partial-summary-frame-v2.md`)
  - Support both structured and LLM content
  - Compact format with key sections

### Phase 6 — Metrics

- [ ] **T6.1:** Implement `CompactionMetrics` (`crates/forge_domain/src/compact/metrics.rs`)
  - Track compaction count, token reduction, strategies used
  - Error recording

- [ ] **T6.2:** Integrate metrics collection into Compactor
  - Record after each compaction

---

## Verification

### Unit Tests
- [ ] Test adaptive eviction window calculation
- [ ] Test importance score calculation
- [ ] Test pre-filter removes short tool results
- [ ] Test deduplication of consecutive tool calls
- [ ] Test LLM summarizer (mocked)

### Integration Tests
- [ ] Test compaction with Extract strategy
- [ ] Test compaction with LLM strategy (mocked)
- [ ] Test compaction with Hybrid strategy
- [ ] Test fallback on LLM failure

### Manual Testing
- [ ] Compact conversation with 50 messages
- [ ] Verify tool call atomicity preserved
- [ ] Verify reasoning chain preserved
- [ ] Compare Extract vs Hybrid output quality

---

## Effort Estimate

| Phase | Tasks | Estimated Hours |
|-------|-------|-----------------|
| Phase 1 | 3 | 4h |
| Phase 2 | 2 | 3h |
| Phase 3 | 3 | 8h |
| Phase 4 | 1 | 2h |
| Phase 5 | 1 | 1h |
| Phase 6 | 2 | 2h |
| **Total** | **12** | **20h** |

---

## Dependencies

- None (self-contained enhancement)

## Blockers

- None identified

---

## Notes

- LLM summarization should use cheap model by default (haiku-3.5)
- All new features gated behind config flags for backward compatibility
- Compaction should still work if LLM provider unavailable (fallback to extract)
