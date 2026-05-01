# ForgeCode Loop & Monitor: Research & Implementation Plan

## Status: Research Complete

## 1. Repository Setup

**Fork:** `KooshaPari/forgecode` (upstream: `tailcallhq/forgecode`)

**Location:** `/Users/kooshapari/CodeProjects/Phenotype/repos/forgecode`

**Remotes:**
- `origin` → `git@github.com:KooshaPari/forgecode.git`
- `upstream` → `git@github.com:tailcallhq/forgecode.git`

**Workspace:** Rust monorepo with 26 crates

---

## 2. Codebase Analysis

### Crate Architecture

```
forgecode/
├── crates/
│   ├── forge_main/      # CLI entry, command parsing, UI
│   ├── forge_app/       # Agent execution, hooks, services
│   ├── forge_api/       # API client
│   ├── forge_domain/    # Domain models
│   ├── forge_services/   # Background services
│   ├── forge_infra/     # Infrastructure (auth, etc.)
│   ├── forge_config/    # Configuration management
│   └── ... (20 more crates)
└── shell-plugin/        # Zsh plugin for : prefix commands
```

### Key Integration Points

#### A. Command System (`crates/forge_main/src/model.rs`)

**Pattern:** `AppCommand` enum defines built-in commands. `ForgeCommandManager` manages all commands.

```rust
// Current built-in commands include:
"agent", "forge", "muse", "sage", "help", "compact", "new", 
"info", "usage", "exit", "update", "dump", "model", ...,
"commit", "config-*", "workspace-*", "skill", "edit", "suggest"
```

**For `$loop` and `$monitor`:** Add new `AppCommand` variants or extend `ForgeCommandManager`.

#### B. Shell Plugin (`shell-plugin/`)

The `:` prefix system handles commands in Zsh. Pattern matching:
```zsh
:BUFFER =~ "^:([a-zA-Z][a-zA-Z0-9_-]*)( (.*))?$"
```

**For `$loop` and `$monitor`:** Add new patterns:
```zsh
\$([a-zA-Z]+)( (.*))?
```

#### C. Hooks System (`crates/forge_app/src/hooks/`)

Existing hook: `doom_loop.rs` - detects repetitive patterns.

**For `$loop` and `$monitor`:** Create new hooks or services that run periodically.

#### D. Conversation Management (`crates/forge_repo/`)

Existing conversation system with `conversation_id` tracking.

**For `$loop`:** Reuse conversation context via `--conversation-id`.

---

## 3. Implementation Strategy

### Option A: Native Rust Implementation (Recommended)

Create new crate: `crates/forge_loop/`

**Structure:**
```
crates/forge_loop/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── scheduler.rs      # Time/schedule management
    ├── executor.rs        # Prompt execution
    ├── state.rs          # Persistence
    └── commands.rs       # $loop, $monitor command handlers
```

**Pros:**
- Native performance
- Single binary distribution
- Full integration with conversation system
- Background execution without external cron

**Cons:**
- More complex implementation
- Needs background task management

### Option B: Shell Plugin + API Integration

Extend shell plugin to handle `$loop` and `$monitor`, with minimal Rust changes.

**Pros:**
- Simpler to implement
- Easier to test incrementally

**Cons:**
- Still relies on external scheduling
- Less native feel

---

## 4. Detailed Implementation Plan

### Phase 1: `$loop` Command

#### 4.1.1 Add Command to Model

**File:** `crates/forge_main/src/model.rs`

```rust
// Add to AppCommand enum:
Loop {
    #[arg(value_name = "INTERVAL")]
    interval: String,
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,
}

/// Start a background loop that executes prompt at interval
fn loop_usage() -> &'static str {
    "$loop <interval> <prompt> - Start autonomous loop"
}
```

#### 4.1.2 Shell Plugin Integration

**File:** `shell-plugin/lib/dispatcher.zsh`

```zsh
# Add pattern for $loop
elif [[ "$BUFFER" =~ "^\$loop( (.*))?$" ]]; then
    local interval_params="${match[1]}"
    # Parse and route to Rust handler
```

#### 4.1.3 Loop Service

**File:** `crates/forge_loop/src/scheduler.rs`

```rust
pub struct LoopScheduler {
    intervals: HashMap<LoopId, LoopConfig>,
    conversation_manager: ConversationManager,
}

impl LoopScheduler {
    pub fn start_loop(&mut self, interval: Duration, prompt: String, conv_id: ConversationId) -> LoopId;
    pub fn stop_loop(&mut self, id: LoopId);
    pub fn list_loops(&self) -> Vec<LoopStatus>;
}
```

### Phase 2: `$monitor` Command

#### 4.2.1 Condition Types

```rust
pub enum MonitorCondition {
    At(Time),                    // $monitor at 09:00
    Every(Duration),             // $monitor every 15m
    WhenFileChanged(PathBuf),    // $monitor when file X
    WhenGitEvent(GitEvent),      // $monitor when git push
    Composite(Vec<Condition>),   // AND/OR combinations
}
```

#### 4.2.2 File Watcher Integration

Use existing `forge_fs` or `forge_walker` crates for file change detection.

#### 4.2.3 Git Hook Integration

Extend existing git hooks system.

---

## 5. Data Model

### Loop State (`~/.forge/loop/state.json`)

```json
{
  "version": 1,
  "loops": [
    {
      "id": "uuid-v4",
      "conversation_id": "conv-uuid",
      "interval_minutes": 5,
      "prompt": "continue working...",
      "status": "running",
      "created_at": "2026-04-30T12:00:00Z",
      "next_run": "2026-04-30T12:05:00Z",
      "last_run": null
    }
  ],
  "monitors": [
    {
      "id": "uuid-v4",
      "conversation_id": "conv-uuid",
      "condition": {
        "type": "time",
        "expression": "at 09:00"
      },
      "prompt": "send standup",
      "status": "paused",
      "last_triggered": null
    }
  ]
}
```

---

## 6. CLI Usage

### User-Facing Commands

```bash
# Loop
$loop 5m "continue work on feature X"
$loop 10m "check CI and report"
$loop status
$loop stop
$loop stop <id>

# Monitor
$monitor at 09:00 "standup"
$monitor every 30m "check PRs"
$monitor when file src/main.rs "run tests"
$monitor when git push "notify team"
$monitor status
$monitor pause <id>
$monitor resume <id>
$monitor stop <id>
```

---

## 7. Verification Criteria

- [ ] `$loop <interval> "<prompt>"` starts a background loop
- [ ] Loop executes prompt at specified interval
- [ ] Conversation context persists across loop executions
- [ ] `$loop status` shows all active loops
- [ ] `$loop stop` gracefully stops loops
- [ ] `$monitor` conditions trigger correctly
- [ ] File change monitoring works
- [ ] Time-based triggers work
- [ ] Commands appear in help/completion
- [ ] State persists across Forge restarts

---

## 8. Effort Estimate

| Component | Effort |
|-----------|--------|
| Command registration | 1 day |
| Loop scheduler service | 3 days |
| Monitor conditions | 3 days |
| Shell plugin integration | 2 days |
| State persistence | 1 day |
| Testing | 2 days |
| Documentation | 1 day |
| **Total** | **~13 days** |

---

## 9. References

- Claude Code `$loop` command behavior
- Existing `doom_loop.rs` hook (reference implementation)
- `ForgeCommandManager` (command registration pattern)
- `shell-plugin/lib/dispatcher.zsh` (command routing)
- Plans directory for feature implementation patterns

---

## 10. Next Steps

1. [ ] Create feature branch: `feat/native-loop-monitor`
2. [ ] Add `crates/forge_loop/` crate structure
3. [ ] Implement core scheduler
4. [ ] Add command registration
5. [ ] Integrate shell plugin
6. [ ] Add state persistence
7. [ ] Write tests
8. [ ] Create PR to upstream
