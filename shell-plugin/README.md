# Forge ZSH Plugin

A powerful ZSH plugin that provides intelligent command transformation, file tagging, and conversation management for the Forge AI assistant.

## Features

- **Smart Command Transformation**: Convert `:command` syntax into forge executions
- **Agent Selection**: Tab completion for available agents using `:agent_name`
- **File Tagging**: Interactive file selection with `@[filename]` syntax
- **Syntax Highlighting**: Visual feedback for commands and tagged files
- **Conversation Continuity**: Automatic session management across commands
- **Interactive Completion**: Fuzzy finding for files and agents via built-in picker

## Prerequisites

Before using this plugin, ensure you have the following tools installed:

- **fd** - Fast file finder (alternative to find)
- **forge** - The Forge CLI tool

### Installation of Prerequisites

```bash
# macOS (using Homebrew)
brew install fd

# Ubuntu/Debian
sudo apt install fd-find

# Arch Linux
sudo pacman -S fd
```

## Usage

### Starting a Conversation

Begin any command with `:` followed by your prompt:

```bash
: Get the current time
```

This automatically starts a new conversation with the default Forge agent.

### Using Specific Agents

Specify an agent by name after the colon:

```bash
:sage How does caching work in this system?
:muse Create a deployment strategy for my app
```

**Tab Completion**: Type `:` followed by partial agent name and press `TAB` for interactive selection.

### File Tagging

Tag files in your commands using the `@[filename]` syntax:

```bash
: Review this code @[src/main.rs]
: Explain the configuration in @[config.yaml]
```

**Interactive Selection**: Type `@` and press `TAB` to search and select files interactively using fuzzy finder.

### Conversation Continuity

Commands within the same session maintain context:

```bash
# First command
: My project uses React and TypeScript

# Second command (remembers previous context)
: How can I optimize the build process?
```

The plugin automatically manages conversation IDs to maintain context across related commands.

### Command Naming

Shell commands should follow the **Object-Action** format.

Examples:
- `:provider-login`
- `:sync-status`

For backward compatibility, `:login` remains available as an alias for `:provider-login`.

### Session Management

#### Starting New Sessions

Clear the current conversation context and start fresh:

```bash
:new
# or use the alias
:n
```

This will:

- Clear the current conversation ID
- Show the banner with helpful information
- Reset the session state
- Display a confirmation message with timestamp

#### System Information

View system and project information:

```bash
:info
# or use the alias
:i
```

This displays:

- System information
- Project details
- Current configuration

- Current configuration

#### Switching Conversations

Browse and switch between conversations interactively:

```bash
:conversation
# or use the alias
:c
```

This will display an interactive list of all conversations with preview, allowing you to select and switch.

Switch to a specific conversation by ID:

```bash
:conversation <conversation_id>
```

Toggle between current and previous conversation (like `cd -`):

```bash
:conversation -
# or
:c -
```

The plugin remembers your previous conversation, allowing you to quickly toggle back and forth. This works just like `cd -` in your shell, and **also works with `:new`** - when you start a new conversation, you can toggle back to your previous one.

If there's no previous conversation tracked (e.g., first time using the plugin), `:c -` will show the conversation list popup, allowing you to select a conversation.

This is useful when:
- You need to temporarily check another conversation and come back
- You're comparing or referencing information between two conversations
- You want to quickly switch context between related tasks
- You started a new conversation but want to reference the previous one

#### Cloning Conversations

Create a copy of an existing conversation with interactive selection:

```bash
:clone
```

This will:
- Display an interactive list of all conversations with preview
- Allow you to select a conversation to clone
- Create a new conversation with the same content
- Automatically switch to the cloned conversation
- Show the cloned conversation content and details

You can also clone a specific conversation by providing its ID:

```bash
:clone <conversation_id>
```

This is useful when you want to:
- Create a backup before making significant changes
- Start a new conversation branch from an existing context
- Experiment with different approaches while preserving the original

#### Session Status

The plugin automatically displays session information including:
- Conversation ID when starting new sessions
- Active agent information
- New session confirmations with timestamps

## Syntax Highlighting

The plugin provides visual feedback through syntax highlighting:

- **Tagged Files** (`@[filename]`): Displayed in **green bold**
- **Agent Commands** (`:agent`): Agent names in **yellow bold**
- **Command Text**: Remaining text in **white bold**

## Configuration

Customize the plugin behavior by setting these variables before loading the plugin:

```bash
# Custom forge binary location
export FORGE_BIN="/path/to/custom/forge"
```

### Available Configuration Variables

- `FORGE_BIN`: Path to the forge executable (default: `forge`)
- `FORGE_EDITOR`: Editor command to use for `:edit` command (default: `$EDITOR` or `nano`)
- `FORGE_SYNC_ENABLED`: Enable/disable automatic workspace sync (default: `true`)
- `FORGE_MAX_COMMIT_DIFF`: Maximum diff size for commit message generation in bytes (default: `100000`)
- `FORGE_SKIP_INTERACTIVE`: Skip interactive prompts (internal use)
- `FORGE_CURRENCY_SYMBOL`: Currency symbol for cost display in ZSH theme (default: `"$"`)
- `FORGE_CURRENCY_CONVERSION_RATE`: Conversion rate for currency display (default: `1.0`)
- `NERD_FONT`: Enable Nerd Font icons in ZSH theme (default: auto-detected, set to `"1"` or `"true"` to enable, `"0"` or `"false"` to disable)
- `USE_NERD_FONT`: Alternative variable for enabling Nerd Font icons (same behavior as `NERD_FONT`)
- Internal pattern matching for conversation syntax (`:`)
- New session command keyword: `:new` or `:n`

### Codebase Indexing

Sync your codebase for semantic search:

```bash
:sync
```

This will index the current directory for semantic code search.

### Environment Diagnostics

Run comprehensive environment diagnostics to check your Forge setup:

```bash
:doctor
```

This will check:
- ZSH version and terminal information
- Forge installation and version
- Plugin and theme loading status
- Completions availability
- Dependencies (fd, bat)
- ZSH plugins (autosuggestions, syntax-highlighting)
- Editor configuration and PATH setup
- Nerd Font support for icons

### .forge Directory

The plugin creates a `.forge` directory in your current working directory (similar to `.git`) for temporary files:

- `FORGE_EDITMSG.md`: Temporary file used when opening an external editor with `:edit`

## Advanced Features

### Command History

All transformed commands are properly saved to ZSH history, allowing you to:
- Navigate command history with arrow keys
- Search previous forge commands with `Ctrl+R`
- Reuse complex commands with file tags

### Keyboard Shortcuts

- **Tab**: Interactive completion for files (`@`) and agents (`:`)
- **Enter**: Transform and execute `:commands`
- **Ctrl+C**: Interrupt running forge commands

## Examples

### Basic Usage

```bash
: What's the weather like?
:sage Explain the MVC pattern
:planner Help me structure this project
```

### With File Tagging

```bash
: Review this implementation @[src/auth.rs]
: Debug the issue in @[logs/error.log] @[config/app.yml]
```

### Session Flow

```bash
: I'm working on a Rust web API
: What are the best practices for error handling?
: Show me an example with @[src/errors.rs]
:info
:new
: New conversation starts here
```


### Codebase Indexing

```bash
# Sync current directory for semantic search
:sync
```