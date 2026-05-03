# ADR-0001: Compaction Summarization Strategy

**Date:** 2026-05-02
**Status:** Accepted
**Deciders:** Forgecode Team

---

## Context

The forgecode context compaction system currently uses pure structural extraction to summarize conversations. This approach:

- Extracts tool calls, tool results, file paths, and commands
- Renders into a markdown template (`forge-partial-summary-frame.md`)
- Is fast (~0ms), deterministic, and cost-free

However, this approach has limitations:
1. **Low semantic fidelity** — captures structure, not meaning
2. **No understanding of decisions** — can't capture why changes were made
3. **Verbose output** — includes all operations, even low-value ones
4. **No prioritization** — treats all content equally

As forgecode grows more capable and handles complex multi-step tasks, the quality of context summarization directly impacts downstream task performance.

---

## Decision

We will implement a **hybrid summarization strategy** with three modes:

```rust
pub enum SummarizationStrategy {
    /// Pure structural extraction (current behavior)
    Extract,

    /// LLM-based semantic summarization
    Llm,

    /// Hybrid: extract first, then refine with LLM
    Hybrid,
}
```

**Default:** `Extract` (backward compatible)
**Configuration:** Per-agent via `compact.summarization_strategy`

---

## Rationale

### Why not pure LLM?

- **Latency**: LLM summarization adds 500ms-2s per compaction
- **Cost**: Per-token API costs accumulate with frequent compaction
- **Determinism**: Same input may produce different outputs
- **Complexity**: Requires error handling for API failures

### Why not pure extraction?

- **Semantic fidelity**: Can't capture decision rationale
- **Noise**: Includes low-value operations
- **Quality ceiling**: Limited improvement potential

### Why hybrid?

- **Best of both**: Fast extraction with LLM refinement
- **Progressive enhancement**: Users can opt into higher quality
- **Fallback safety**: Extract always available as fallback
- **Cost control**: Use cheaper models for summarization

---

## Implementation Options

### Option A: Extract-Only (Status Quo)

**Pros:**
- Fastest (~0ms)
- Zero API cost
- Fully deterministic
- No API failure modes

**Cons:**
- Low semantic fidelity
- Verbose summaries
- No decision capture

### Option B: Pure LLM

**Pros:**
- Highest semantic fidelity
- Captures decisions and rationale
- Can identify important context

**Cons:**
- ~500ms-2s latency per compaction
- Per-token API cost
- Non-deterministic output
- API failure handling required

### Option C: Hybrid (Selected)

**Pros:**
- Balance of speed and quality
- Can use cheap models (haiku)
- Structured data from extraction + semantics from LLM
- Fallback to extract on failure

**Cons:**
- More complex implementation
- Two-step process adds some latency
- Requires LLM integration

### Option D: Adaptive Cascade

**Pros:**
- Automatically chooses strategy based on complexity
- Best resource allocation
- Can escalate as needed

**Cons:**
- Most complex implementation
- Harder to reason about behavior
- More configuration surface

---

## Decision Outcome

We select **Option C (Hybrid)** as the default for enhanced compaction, with:

1. **Extract as default** for backward compatibility
2. **Hybrid mode** as the recommended upgrade path
3. **LLM-only** available as opt-in for users who prioritize quality over speed
4. **Configurable model** for summarization (default: haiku-3.5)
5. **Timeout protection** (3s max for LLM operations)
6. **Fallback to extract** on any LLM failure

---

## Consequences

### Positive

- [x] Improved summary quality when enabled
- [x] Backward compatible with existing configurations
- [x] Users can choose their cost/quality tradeoff
- [x] Can use cheap models for summarization
- [x] Fallback ensures reliability

### Negative

- [ ] Adds complexity to Compactor implementation
- [ ] Requires LLM provider integration in forge_app
- [ ] Template engine needs enhancement for new formats

### Neutral

- [ ] New configuration options added (non-breaking)
- [ ] Metrics collection added for observability
- [ ] History tracking for incremental summarization

---

## Configuration

```yaml
# forge.toml
[compact]
enabled = true
token_threshold = 100_000
eviction_window = 0.2

# NEW: Summarization configuration
summarization_strategy = "hybrid"  # extract | llm | hybrid
summary_model = "claude-3-5-haiku"  # cheaper model for summarization
summary_max_tokens = 4000
summary_timeout_secs = 3
```

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| LLM adds latency | High | Medium | Use cheap model, timeout, cache summaries |
| LLM quality inconsistent | Medium | High | Validate format, fallback to extract |
| API failures | Low | Medium | Graceful fallback to extract |
| Cost accumulation | Medium | Medium | Per-compaction budget, cheap models |

---

## Review History

- 2026-05-02: Initial draft
- 2026-05-02: Accepted (selecting Option C)

---

## Related Documents

- Plan: `plans/2026-05-02-compaction-enhancement-v1.md`
- Config: `crates/forge_config/src/compact.rs`
- Domain: `crates/forge_domain/src/compact/`
- App: `crates/forge_app/src/compact.rs`
