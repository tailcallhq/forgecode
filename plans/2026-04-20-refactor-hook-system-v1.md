# Refactor Hook System for Elegance and Add Thinking Support

## Objective

Refactor the hook system to separate the **observer** pattern (read-only event hooks) from the **interceptor** pattern (mutable tool call rewriting), eliminate the ugly destructuring in `orch.rs`, and add a proper thinking/reasoning hook that injects reasoning into the conversation.

The core insight: `EventHandle<T>` currently takes `&mut T` for ALL handlers, but only `ExternalHookHandler` actually mutates the event payload. The other 5 handlers only mutate the `Conversation` (which is correct and necessary). This plan separates these concerns cleanly.

---

## Current State Analysis

### Mutation Audit of All Handlers

| Handler | Event Param | What It Does With Event | What It Does With Conversation |
|---------|------------|------------------------|-------------------------------|
| `TracingHandler` (all 6 events) | Read-only logging | Nothing | Nothing |
| `CompactionHandler` (Response) | Ignores (`_event`) | Replaces `conversation.context` | 
| `DoomLoopDetector` (Request) | Read-only | Appends messages to `conversation.context` |
| `PendingTodosHandler` (End) | Ignores (`_event`) | Appends messages to `conversation.context` |
| `TitleGenerationHandler` (Start/End) | Read-only | Sets `conversation.title` |
| **`ExternalHookHandler` (ToolcallStart)** | **Mutates `event.payload.tool_call.arguments`** | Nothing |

**Key finding**: Only `ExternalHookHandler` mutates the event payload. All other handlers either read the event or ignore it entirely. They all legitimately mutate `Conversation`.

### The Ugly Code (orch.rs:128-134)

```rust
let updated_tool_call =
    if let LifecycleEvent::ToolcallStart(data) = toolcall_start_event {
        data.payload.tool_call
    } else {
        (*tool_call).clone()
    };
```

This exists because the hook fires a `LifecycleEvent` enum, and the orchestrator must destructure it to get back the (potentially mutated) tool call. This is a code smell — the enum wrapping serves the dispatch system, not the caller.

---

## Implementation Plan

### Phase 1: Revert EventHandle to Immutable `&T`

- [ ] **1.1. Change `EventHandle::handle` signature from `&mut T` to `&T`**
  - In `crates/forge_domain/src/hook.rs:134`, change `event: &mut T` to `event: &T`
  - Rationale: Only 1 of 6 handlers mutates the event. The clean API should be immutable by default.

- [ ] **1.2. Update `Hook`'s `EventHandle<LifecycleEvent>` impl to use `&self` dispatch**
  - In `hook.rs:326-345`, change the match arms to pass `&T` instead of `&mut T`
  - The `EventHandle<LifecycleEvent> for Hook` impl will pass `data` as `&EventData<P>` to each slot

- [ ] **1.3. Update `CombinedHandler` to pass `&T`**
  - In `hook.rs:356-363`, change both handler calls from `&mut T` to `&T`

- [ ] **1.4. Update `NoOpHandler` to accept `&T`**
  - In `hook.rs:374`, change `_: &mut T` to `_: &T`

- [ ] **1.5. Update the blanket `Fn` impl for `EventHandle`**
  - In `hook.rs:380-388`, change the closure signature from `Fn(&mut T, &mut Conversation)` to `Fn(&T, &mut Conversation)`

- [ ] **1.6. Update the `Box<dyn EventHandle<T>>` impl**
  - In `hook.rs:168-172`, change to pass `&T`

- [ ] **1.7. Update `EventHandleExt::and` to work with `&T` signature**
  - In `hook.rs:152-164`, ensure the return type still boxes correctly

- [ ] **1.8. Update all 6 handler implementations to accept `&T` instead of `&mut T`**
  - `TracingHandler` (all 6 impls in `hooks/tracing.rs`) — change `event: &mut EventData<...>` to `event: &EventData<...>`
  - `CompactionHandler` in `hooks/compaction.rs:32` — already ignores event, just change signature
  - `DoomLoopDetector` in `hooks/doom_loop.rs` — reads event only
  - `PendingTodosHandler` in `hooks/pending_todos.rs:43` — already ignores event
  - `TitleGenerationHandler` in `hooks/title_generation.rs:35,65` — reads event only
  - `ExternalHookHandler` in `hooks/external.rs:54` — **remove this impl entirely** (moved to interceptor in Phase 2)

- [ ] **1.9. Update all event firing sites in `orch.rs` to use `&T`**
  - `orch.rs:248-255` (Start event) — remove `mut` from `start_event`
  - `orch.rs:275-282` (Request event) — remove `mut` from `request_event`
  - `orch.rs:315-322` (Response event) — remove `mut` from `response_event`
  - `orch.rs:119-126` (ToolcallStart event) — **remove entirely**, replaced by interceptor call in Phase 2
  - `orch.rs:143-150` (ToolcallEnd event) — remove `mut` from `toolcall_end_event`
  - `orch.rs:424-431` (End event) — remove `mut` from `end_event`

- [ ] **1.10. Update all tests in `hook.rs` tests module (lines 401-1087)**
  - Change all closure signatures from `&mut EventData<...>` to `&EventData<...>`
  - Remove `mut` from event variables where only `&T` is needed
  - Approximately 15 test functions to update

- [ ] **1.11. Update handler-specific tests**
  - `hooks/tracing.rs:155-247` — change `&mut` to `&` in test handler calls
  - `hooks/doom_loop.rs:253-784` — update test closures
  - `hooks/pending_todos.rs:134-272` — update test closures
  - `hooks/title_generation.rs:120-310` — update test closures

- [ ] **1.12. Update integration tests in `orch_spec/`**
  - Search for all `&mut` event handler patterns and update to `&`

### Phase 2: Introduce `ToolCallInterceptor` Trait

- [ ] **2.1. Define the `ToolCallInterceptor` trait in `crates/forge_domain/src/hook.rs`**
  - New trait:
    ```rust
    #[async_trait]
    pub trait ToolCallInterceptor: Send + Sync {
        async fn intercept(&self, tool_call: &mut ToolCallFull, agent: &Agent, model_id: &ModelId) -> anyhow::Result<()>;
    }
    ```
  - Rationale: This is a focused trait for the single case of tool call mutation. It takes `&mut ToolCallFull` directly — no enum wrapping, no destructuring.

- [ ] **2.2. Add a `tool_call_interceptor` field to `Hook`**
  - Add `tool_call_interceptor: Option<Box<dyn ToolCallInterceptor>>` to the `Hook` struct
  - Default is `None` (no interception)
  - Add a builder method `pub fn interceptor(mut self, interceptor: impl ToolCallInterceptor + 'static) -> Self`

- [ ] **2.3. Add a `run_interceptor` method to `Hook`**
  - Method: `pub async fn run_interceptor(&self, tool_call: &mut ToolCallFull, agent: &Agent, model_id: &ModelId) -> anyhow::Result<()>`
  - If `self.tool_call_interceptor` is `Some`, call it; otherwise no-op
  - This is called separately from event hooks

- [ ] **2.4. Refactor `ExternalHookHandler` to implement `ToolCallInterceptor`**
  - In `hooks/external.rs`, remove the `EventHandle<EventData<ToolcallStartPayload>>` impl
  - Implement `ToolCallInterceptor` instead:
    ```rust
    #[async_trait]
    impl ToolCallInterceptor for ExternalHookHandler {
        async fn intercept(&self, tool_call: &mut ToolCallFull, agent: &Agent, model_id: &ModelId) -> anyhow::Result<()> {
            // Same logic as before, but operating directly on tool_call
        }
    }
    ```

- [ ] **2.5. Update `Hook::zip` to combine interceptors**
  - When zipping two hooks, if both have interceptors, create a `CombinedInterceptor` that runs both in sequence
  - If only one has an interceptor, use that one
  - If neither has one, keep `None`

- [ ] **2.6. Clean up `orch.rs` execute_tool_calls method**
  - Replace the current ToolcallStart event firing + destructuring pattern (lines 118-134) with:
    1. Fire the `ToolcallStart` event with immutable reference (for tracing/observability)
    2. Run the interceptor separately on the `tool_call` directly
    3. No more enum destructuring
  - The new flow:
    ```rust
    // Fire immutable event for observers (tracing, etc.)
    let toolcall_start_event = LifecycleEvent::ToolcallStart(EventData::new(
        self.agent.clone(),
        self.agent.model.clone(),
        ToolcallStartPayload::new((*tool_call).clone()),
    ));
    self.hook.handle(&toolcall_start_event, &mut self.conversation).await?;

    // Run interceptor if present (may mutate tool_call)
    let mut updated_tool_call = (*tool_call).clone();
    self.hook.run_interceptor(&mut updated_tool_call, &self.agent, &self.agent.model).await?;

    // Execute the tool
    let tool_result = self.services.call(&self.agent, tool_context, updated_tool_call).await;
    ```

- [ ] **2.7. Update `ForgeApp` wiring in `app.rs`**
  - Remove `external_handler` from the `.on_toolcall_start()` chain
  - Add `.interceptor(external_handler)` to the hook builder instead
  - The `on_toolcall_start` slot will only have `TracingHandler` (or no handler at all)

### Phase 3: Add Thinking/Reasoning Support Hook

- [ ] **3.1. Add a `ReasoningPayload` type in `hook.rs`**
  - New payload struct:
    ```rust
    #[derive(Debug, PartialEq, Clone, Setters)]
    #[setters(into)]
    pub struct ReasoningPayload {
        pub reasoning: String,
        pub reasoning_details: Option<Vec<ReasoningFull>>,
    }
    ```
  - This carries the full reasoning content from the LLM response

- [ ] **3.2. Add `Reasoning` variant to `LifecycleEvent`**
  - `Reasoning(EventData<ReasoningPayload>)` — fired after a response that contains reasoning content
  - This allows handlers to observe, log, or react to reasoning content

- [ ] **3.3. Add `on_reasoning` slot to `Hook`**
  - `on_reasoning: Box<dyn EventHandle<EventData<ReasoningPayload>>>`
  - Add corresponding builder method `pub fn on_reasoning(mut self, handler: impl EventHandle<EventData<ReasoningPayload>> + 'static) -> Self`
  - Update `Hook::default()`, `Hook::new()`, `Hook::zip()`, and the `EventHandle<LifecycleEvent>` match

- [ ] **3.4. Fire the Reasoning event in `orch.rs`**
  - After the Response event (around `orch.rs:314-322`), if `message.reasoning.is_some()`, fire a `Reasoning` event
  - This gives handlers access to reasoning content without coupling to the response payload

- [ ] **3.5. Implement `TracingHandler` for `ReasoningPayload`**
  - Add `EventHandle<EventData<ReasoningPayload>>` impl to `TracingHandler`
  - Log reasoning content at debug level for observability

- [ ] **3.6. Consider a `ReasoningInjectionHandler` (optional, future work)**
  - A handler that could inject reasoning summaries into the conversation context
  - This is not part of the initial refactor but the hook infrastructure enables it
  - Document this as a follow-up opportunity

### Phase 4: Update Exports and Module Structure

- [ ] **4.1. Export `ToolCallInterceptor` from `forge_domain`**
  - Add to `crates/forge_domain/src/lib.rs` public exports
  - Ensure it's re-exported from `hook` module

- [ ] **4.2. Export `ReasoningPayload` from `forge_domain`**
  - Add to public exports alongside other payload types

- [ ] **4.3. Update `hooks/mod.rs` if needed**
  - No new handler files needed — `ExternalHookHandler` stays in `external.rs`, just changes trait impl

### Phase 5: Update Tests

- [ ] **5.1. Update all unit tests in `hook.rs` to use `&T` signatures**
  - All closure-based handlers in tests need `&EventData<...>` instead of `&mut EventData<...>`
  - Remove unnecessary `mut` from event variables

- [ ] **5.2. Add tests for `ToolCallInterceptor`**
  - Test that `Hook::run_interceptor` calls the interceptor
  - Test that `Hook::default().run_interceptor()` is a no-op
  - Test interceptor composition via `zip`

- [ ] **5.3. Add tests for `Reasoning` lifecycle event**
  - Test that the `Reasoning` event fires when response has reasoning content
  - Test that `Hook::default()` handles `Reasoning` event (no-op)
  - Test that a custom handler receives the reasoning payload

- [ ] **5.4. Update `ExternalHookHandler` tests (if any exist)**
  - Currently no tests for this handler (it relies on filesystem)
  - Consider adding a basic unit test with a mock script path

- [ ] **5.5. Run full test suite**
  - `cargo insta test --accept` in `forge_domain`
  - `cargo insta test --accept` in `forge_app`
  - Verify all tests pass with `cargo check` and `cargo build`

---

## Verification Criteria

- [ ] `EventHandle::handle` takes `&T` (immutable) — no handler can accidentally mutate event data
- [ ] `ToolCallInterceptor` is a separate trait with `&mut ToolCallFull` — only tool-call-mutating code implements this
- [ ] `orch.rs` has no `if let LifecycleEvent::ToolcallStart(data) = ...` destructuring pattern
- [ ] `ExternalHookHandler` implements `ToolCallInterceptor`, not `EventHandle<EventData<ToolcallStartPayload>>`
- [ ] `TracingHandler` still works for all events including the new `Reasoning` event
- [ ] All existing tests pass without modification to test assertions (only signature changes)
- [ ] `cargo check` and `cargo build` succeed with no warnings
- [ ] `cargo insta test --accept` passes for `forge_domain` and `forge_app`

## Potential Risks and Mitigations

1. **Risk: Breaking change to `EventHandle` trait signature**
   - Mitigation: All implementations are internal to the crate. The trait is not a public API boundary — it's used within `forge_app` and `forge_domain` only. Update all impls atomically.

2. **Risk: The `Fn` blanket impl change breaks existing closure-based handlers**
   - Mitigation: Search for all `Fn(&mut` patterns in the codebase and update them. The closure impl is only used in tests and the `From<F> for Box<dyn EventHandle<T>>` conversion.

3. **Risk: Adding `Reasoning` variant to `LifecycleEvent` breaks exhaustive match statements**
   - Mitigation: Search for all `match` on `LifecycleEvent` and add the new variant. There's only one match site in `Hook`'s `EventHandle<LifecycleEvent>` impl.

4. **Risk: The `ToolCallInterceptor` approach requires the orchestrator to clone the tool call**
   - Mitigation: The current code already clones (`(*tool_call).clone()`). The interceptor operates on the clone, which is then used for execution. No additional cloning overhead.

5. **Risk: Removing `ExternalHookHandler` from `on_toolcall_start` changes tracing behavior**
   - Mitigation: The `TracingHandler` still fires on `ToolcallStart` for logging. The interceptor runs separately. The tracing log will show the *original* arguments, not the modified ones — this is arguably better for auditability.

## Alternative Approaches

1. **Keep `&mut T` but add a separate `observe(&self, event: &T, ...)` method**: Less invasive but doesn't solve the fundamental API clarity issue. The `&mut` would still be misleading for 95% of handlers.

2. **Use a `Cow<T>` or `RefCell<T>` approach**: Overly complex for what is fundamentally a design clarity issue. Adds runtime overhead and cognitive complexity.

3. **Make `ToolCallInterceptor` a generic trait over any event type**: Overly abstract. There's only one event type that needs mutation (tool calls). A specific trait is clearer and more discoverable.

4. **Use an `Arc<Mutex<ToolCallFull>>` shared between hook and orchestrator**: Would avoid the destructuring but introduces shared mutable state and potential deadlocks. The interceptor approach is simpler and more explicit.

## Assumptions

- The `Conversation` mutation (second parameter) should remain `&mut Conversation` — handlers legitimately need to modify conversation state (compaction, doom loop reminders, title setting, todo injection).
- The `Reasoning` lifecycle event is informational only — handlers observe it but don't need to mutate reasoning content.
- The `ToolCallInterceptor` runs synchronously with the tool execution flow — no async composition needed beyond what `async_trait` provides.
- The `Hook::zip` method should compose interceptors similarly to how it composes event handlers.
