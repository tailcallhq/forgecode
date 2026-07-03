# RAM Investigation: forge-dev 3+GB Memory Usage

**Date**: 2026-06-29  
**Scope**: Read-only analysis of `forge-dev` (forgecode fork) TUI conversation list  
**Symptom**: 2585 conversations shown; list is laggy; process RSS > 3GB  

---

## Executive Summary

The root cause is a **single `SELECT *` query that loads full conversation `context` blobs** into memory for every conversation in the list view. The `ConversationRecord` Diesel struct maps all 22 table columns, including `context` (plain JSON, large) and `context_zstd` (compressed, up to several MB when decompressed). With ~2585 conversations and a DB of ~6.5GB, the `context`/`context_zstd` columns dominate RAM consumption. The TUI then deserialises **every row's context** (JSON → `Context` struct with full `Vec<MessageEntry>`) even though the list selector only needs `title`, `id`, `created_at`, `updated_at`, `message_count`, and `cwd`.

**Top RAM cause**: Loading and deserialising full `Context` blobs for every conversation in the list query.  
**Estimated per-conversation overhead**: 100 KB–3 MB (depending on history length) × 2585 = **~260 MB–7.8 GB** in heap.

---

## Top 3 Memory Hogs

### 1. `SELECT *` loads `context`/`context_zstd` into memory for every row (PRIMARY)

| File | Line(s) | Issue |
|---|---|---|
| `crates/forge_repo/src/conversation/conversation_repo.rs` | 262–290 | `get_parent_conversations()` uses Diesel's `.load()` which issues `SELECT *` and maps to `ConversationRecord` (22 columns, including `context: Option<String>` and `context_zstd: Option<Vec<u8>>`) |
| `crates/forge_repo/src/conversation/conversation_record.rs` | 950–969 | `ConversationRecord` struct declares `context` + `context_zstd` + `is_compressed` |
| `crates/forge_domain/src/context.rs` | 401–409 | `Context` struct contains `messages: Vec<MessageEntry>` — the real bulk (hundreds of messages per conversation) |
| `crates/forge_repo/src/conversation/conversation_record.rs` | 1113–1135 | `TryFrom<ConversationRecord>` decompresses zstd *and* deserialises into full `Context` object for *every* row in the list |

**Impact**: Even if `max_conversations` is set to a moderate value, the full `Context` for each row is deserialised into a Rust struct in memory before the TUI ever renders a single row. Diesel's `Queryable` derive requires `SELECT *`; there is no lite/select-columns variant.

### 2. `user_initiated_conversations` forces full `Context` load (SECONDARY)

| File | Line(s) | Issue |
|---|---|---|
| `crates/forge_main/src/ui.rs` | 2712–2728 | Filters out agent-initiated conversations by accessing `conversation.context.as_ref().and_then(|c| c.initiator.as_deref())` |
| `crates/forge_main/src/ui.rs` | 1016–1020, 2178–2183, 2265–2271, 3282–3285 | Every call site passes `Vec<Conversation>` (with full context) to `user_initiated_conversations()` |

**Impact**: The `initiator` field (a single string) lives inside `Context`, which lives inside the same struct as `messages: Vec<MessageEntry>`. To read `initiator`, the entire Context including all messages must be deserialised. There is no `initiator` column on the SQLite table, so it cannot be queried without loading the blob.

### 3. TUI builds an in-memory `HashMap` of all conversations with `Arc` clones (TERTIARY)

| File | Line(s) | Issue |
|---|---|---|
| `crates/forge_main/src/conversation_selector.rs` | 186–209 | After the API returns `Vec<Conversation>` (already bloated), the selector creates a `HashMap<String, Arc<Conversation>>` with an `Arc::new((*c).clone())` for every conversation — **doubling** the heap footprint of the context blobs |
| `crates/forge_main/src/conversation_selector.rs` | 94–109 | The selector also holds a `Vec<&Conversation>` (borrowed references), plus `Vec<SelectRow>` with cloned strings |

**Impact**: The context blobs exist in memory at three levels:
1. The `Vec<Conversation>` returned by the API
2. The `Vec<&Conversation>` borrowed slice (zero-cost)
3. The `HashMap<String, Arc<Conversation>>` — **each Conversation is cloned and Arc-wrapped**

On a 2.5K-item list this triples the per-conversation memory cost of the context blobs.

---

## Fix Plan (headlines)

### P0 — List query selects only metadata columns (no context blobs)

**Files to modify**:
- `crates/forge_repo/src/conversation/conversation_repo.rs:262–290`
- `crates/forge_repo/src/conversation/conversation_record.rs:950–969` (add a `ConversationRecordLite`)
- `crates/forge_domain/src/conversation.rs:41–63` (add a `ConversationSummary` domain struct)

**Approach**: Add a new Diesel query that explicitly selects only metadata columns (`conversation_id`, `title`, `workspace_id`, `created_at`, `updated_at`, `parent_id`, `source`, `cwd`, `message_count`) using `.select()`. Map to a lightweight struct that excludes `context`/`context_zstd`. The domain type `ConversationSummary` holds only fields needed for the selector display.

### P0 — Add `initiator` column to the conversations table

**Files to modify**:
- Database migration (add column or extract during write)
- `crates/forge_repo/src/conversation/conversation_record.rs` — populate on write
- `crates/forge_main/src/ui.rs:2712–2728` — use `initiator` column instead of `context.initiator`

**Approach**: Add an `initiator` TEXT column to the `conversations` SQLite table. Set it during `INSERT`/`UPDATE` from `context.initiator`. This eliminates the need to deserialise the full `Context` just to filter user-vs-agent conversations.

### P1 — Lazy-load context only on conversation open

**Approach**: The conversation selector should work with `ConversationSummary` objects. Only when the user selects a conversation and `on_show_last_message` (or the `/clone`/`/rename` preview shell command fires) should the full `Conversation` be fetched via `get_conversation(id)`. This moves the 6.5GB of context blobs off the hot path.

### P1 — Remove the `HashMap<String, Arc<Conversation>>` clone in the selector

**Approach**: The selector only needs the `ConversationId` of the selected item, not a full `Arc<Conversation>`. Change `select_conversation` to return `Option<ConversationId>`, then do a second round-trip to fetch the full `Conversation` only when needed.

### P2 — TUI list virtualization (paginated scroll)

**Approach**: The `ForgeWidget` (in `crates/forge_select/`) could implement virtual scrolling — only render `N + buffer` rows at a time. This caps the `SelectRow` allocation to ~50 rows regardless of total count. Easiest to implement as a `page_size` parameter that limits DB query at the repo layer and lazy-loads next/prev pages on user scroll.

---

## Offending Query / Code Table

| Component | File | Lines | What it does wrong |
|---|---|---|---|
| DB query | `crates/forge_repo/src/conversation/conversation_repo.rs` | 262–290 | `SELECT *` loads `context` + `context_zstd` for all parent conversations |
| Record mapping | `crates/forge_repo/src/conversation/conversation_record.rs` | 950–969 | 22-column struct includes 2 heavy blob columns |
| Decompress + deserialise | `crates/forge_repo/src/conversation/conversation_record.rs` | 1113–1135 | zstd decode + JSON parse every `context` blob into a `Context` with `Vec<MessageEntry>` |
| API return type | `crates/forge_api/src/forge_api.rs` | 315 | Returns `Vec<Conversation>` (heavy) instead of a lite type |
| TUI filter requires context | `crates/forge_main/src/ui.rs` | 2712–2728 | `user_initiated_conversations` reads `context.initiator` → forces deserialisation |
| Selector clones everything | `crates/forge_main/src/conversation_selector.rs` | 186–209 | Copies every `Conversation` into `HashMap<String, Arc<Conversation>>` |
| No pagination | `crates/forge_main/src/conversation_selector.rs` | 84–209 | Whole list loaded at once; no page_size or virtual scroll |
| Config default is 0 | `forge.schema.json` | 96 | `max_conversations` defaults to `0` (serde default for `usize`), meaning `Some(0)` → `LIMIT 0` → no rows. User must set an explicit large value to see 2585 — or the workshop/config ships a higher default |

---

## Doc Path

`~/CodeProjects/Phenotype/repos/forgecode/docs/sessions/20260629-forgedev-ram/RAM_INVESTIGATION.md`
