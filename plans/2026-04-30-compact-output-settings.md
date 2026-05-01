# ForgeCode Compact/Concise Output Settings

## Context

Following Codex's approach to output verbosity, implement settings for Forge to collapse/compact output by default with user-configurable modes.

## User Requirements

- **Hyper concise mode**: 1 line per tool max, no more
- Anything not verbal output (or input) is the same
- Settings option to change (follow Codex pattern)

## Research Summary (from Codex)

### Codex Pattern

| Setting | Values | Purpose |
|---------|--------|---------|
| `model_verbosity` | `low\|medium\|high` | Controls API output detail |
| `hide_agent_reasoning` | `bool` | Suppress reasoning from output |
| `show_raw_agent_reasoning` | `bool` | Show raw reasoning content |
| `tool_output_token_limit` | `Option<usize>` | Token budget for tool outputs |

### TUI Display

- Compact mode: 1-line-per-tool with expandable details
- Tree-style prefixes (`└`) for nested output
- Truncates long tool outputs to preserve terminal space

## Implementation Plan

### Phase 1: Configuration Schema

**File**: `crates/forge_config/src/config_schema.rs`

Add output settings:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSettings {
    /// Output verbosity level
    #[serde(default = "default_verbosity")]
    pub verbosity: Verbosity,

    /// Compact mode: 1 line per tool
    #[serde(default = "default_true")]
    pub compact: bool,

    /// Show tool headers
    #[serde(default = "default_true")]
    pub show_tool_headers: bool,

    /// Max lines per tool output (0 = unlimited)
    #[serde(default = "default_one")]
    pub max_tool_lines: usize,

    /// Truncate tool output beyond N chars
    #[serde(default = "default_200")]
    pub tool_truncate_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum Verbosity {
    #[default]
    Compact,   // 1 line per tool
    Concise,   // Brief summaries
    Normal,    // Standard output
    Verbose,   // Full details
}
```

### Phase 2: Tool Output Formatting

**File**: `crates/forge_display/src/tool_output.rs`

Create formatter:
```rust
pub struct ToolOutputFormatter {
    settings: OutputSettings,
}

impl ToolOutputFormatter {
    /// Format tool call in compact mode (1 line)
    pub fn format_compact(&self, tool: &ToolCall) -> String {
        // "✓ tool_name: result summary..."
    }

    /// Format tool call in normal mode
    pub fn format_normal(&self, tool: &ToolCall) -> Vec<Line> {
        // Full output with truncation
    }

    /// Format tool call in verbose mode
    pub fn format_verbose(&self, tool: &ToolCall) -> Vec<Line> {
        // Complete output
    }
}
```

### Phase 3: UI Integration

**File**: `crates/forge_main/src/ui.rs`

Wire settings into display pipeline:
```rust
async fn display_tool_result(&mut self, tool: ToolCall) -> Result<()> {
    let formatter = ToolOutputFormatter::new(self.output_settings());

    match self.output_settings().verbosity {
        Verbosity::Compact => self.writeln(formatter.format_compact(&tool))?,
        Verbosity::Concise => self.writeln(formatter.format_concise(&tool))?,
        Verbosity::Normal => self.writeln(formatter.format_normal(&tool))?,
        Verbosity::Verbose => self.writeln_verbose(formatter.format_verbose(&tool))?,
    }
    Ok(())
}
```

### Phase 4: CLI/Config Options

**File**: `crates/forge_main/src/cli.rs`

Add CLI flags:
```rust
/// Output verbosity: compact|concise|normal|verbose
#[arg(long, value_enum, default_value_t = Verbosity::Compact)]
pub verbosity: Verbosity,

/// Compact mode: 1 line per tool
#[arg(long)]
pub compact: bool,
```

### Phase 5: Shell Plugin Integration

**File**: `shell-plugin/lib/dispatcher.zsh`

Add `:compact`, `:concise`, `:verbose` commands.

## Files to Modify

| File | Change |
|------|--------|
| `crates/forge_config/src/config_schema.rs` | Add `OutputSettings`, `Verbosity` |
| `crates/forge_display/src/lib.rs` | Export formatter module |
| `crates/forge_display/src/tool_output.rs` | NEW - Tool output formatting |
| `crates/forge_main/src/cli.rs` | Add `--verbosity`, `--compact` flags |
| `crates/forge_main/src/ui.rs` | Wire formatter into display pipeline |
| `shell-plugin/lib/dispatcher.zsh` | Add `:compact`, `:concise`, `:verbose` |

## Verification

```bash
# Test compact mode
forge-dev -v compact -p "list files"

# Test normal mode
forge-dev -v normal -p "list files"

# Test CLI flag
forge-dev --verbosity compact -p "list files"
```
