# Fish Shell 适配 + 粘贴图片功能 实现计划

## Objective

将 Forge 的 shell 插件系统从仅支持 ZSH 扩展为同时支持 Fish shell，使其达到与 ZSH 插件同等的功能水平（`: ` 前缀命令系统、`@[file]` 附件、Tab 补全、右提示栏、语法高亮、终端上下文捕获）。同时研究粘贴图片的可行性方案。

## 背景分析

### 当前 ZSH 插件架构

Forge 的 ZSH 插件由以下模块组成（`shell-plugin/` 目录）：

| 模块 | 文件 | 功能 |
|---|---|---|
| 配置 | `lib/config.zsh` | 全局变量（会话 ID、agent、model 等） |
| 高亮 | `lib/highlight.zsh` | `@[...]` 青色高亮、`:` 命令黄色高亮 |
| 工具函数 | `lib/helpers.zsh` | `_forge_exec`、`_forge_fzf`、日志、后台同步 |
| 上下文捕获 | `lib/context.zsh` | preexec/precmd hooks、OSC 133 标记、命令环形缓冲 |
| 补全 | `lib/completion.zsh` | `@` 文件补全 + `:` 命令补全（fzf） |
| 调度器 | `lib/dispatcher.zsh` | `forge-accept-line` widget、命令路由 |
| 绑定 | `lib/bindings.zsh` | Enter 键绑定、bracketed-paste 处理 |
| 主题 | `forge.theme.zsh` | RPROMPT 显示 agent/model/token 信息 |
| 动作 | `lib/actions/*.zsh` | 各命令的具体实现 |

Rust 端（`crates/forge_main/src/zsh/`）：
- `plugin.rs` — 生成插件脚本、安装到 .zshrc、doctor 诊断
- `paste.rs` — 粘贴文本格式化（路径 → `@[path]`）
- `rprompt.rs` — 右提示栏信息
- `style.rs` — 样式工具
- `mod.rs` — 模块入口

CLI 入口（`crates/forge_main/src/cli.rs`）：
- `ZshCommandGroup` 枚举控制 `forge zsh plugin/theme/doctor/setup/rprompt/format/keyboard` 子命令

### Fish 与 ZSH 的关键差异

| 特性 | ZSH | Fish |
|---|---|---|
| 命令拦截 | ZLE widget（`forge-accept-line`） | `bind` + fish 函数 或 `fish_command_not_found` |
| 变量声明 | `typeset -h` | `set -g` / `set -l` |
| 数组 | `typeset -ha arr=()` | `set -g arr` |
| 正则匹配 | `[[ "$str" =~ "regex" ]]` | `string match -r` |
| Hook 系统 | `preexec_functions` / `precmd_functions` | `fish_preexec` / `fish_prompt` 事件 |
| 补全系统 | ZSH compsys | Fish `complete` 命令 |
| 语法高亮 | `ZSH_HIGHLIGHT_PATTERNS` | Fish 内置语法高亮（不可扩展，但可通过 `fish_color_*` 变量调整） |
| Bracketed paste | 自定义 widget | Fish 内置支持 bracketed paste |
| 右提示栏 | `RPROMPT` | `fish_right_prompt` 函数 |
| 配置文件 | `~/.zshrc` | `~/.config/fish/config.fish` |
| 插件加载 | `source` + oh-my-zsh | Fish 内置函数自动加载 `$fish_function_path` |
| fzf 集成 | 直接调用 | 需要显式 pipe |

### 粘贴图片的可行性

当前 Forge 已有图片处理能力：
- `crates/forge_domain/src/image.rs` — Image 类型（base64 编码）
- `crates/forge_services/src/tool_services/image_read.rs` — 读取 jpg/png/webp/gif
- `crates/forge_services/src/attachment.rs` — 附件系统已支持 `@[image.png]` 自动识别为图片附件

**粘贴图片的技术路径：**

1. **终端剪贴板图片获取**：大多数终端不支持直接将剪贴板中的图片作为二进制数据粘贴到 shell 输入中。bracketed paste 只传递文本。
2. **Kitty Graphics Protocol**：Kitty 终端支持通过 OSC 52 剪贴板协议和文件传输协议读取剪贴板内容，但这是 Kitty 特有的。
3. **可行方案**：
   - **方案 A（推荐）**：用户通过 `:paste-image` 命令触发，Forge 调用系统剪贴板工具（`xclip -selection clipboard -t image/png -o` / `pbpaste` / `wl-paste`）读取剪贴板中的图片，保存为临时文件，自动插入 `@[tempfile.png]`
   - **方案 B**：利用 Kitty 的 clipboard 协议（`OSC 52`）或文件传输协议，从终端获取图片数据
   - **方案 C**：在 TUI 模式（reedline）中拦截 paste 事件，检测是否包含二进制图片数据（需要终端支持）

## Implementation Plan

### Phase 1: Fish Shell 插件基础架构

- [ ] **1.1 创建 Fish 插件目录结构**  
  在 `shell-plugin/` 下创建 `fish/` 子目录，包含与 ZSH `lib/` 对应的 Fish 脚本文件：
  - `fish/config.fish` — 全局变量
  - `fish/helpers.fish` — 工具函数
  - `fish/dispatcher.fish` — 命令调度
  - `fish/completion.fish` — 补全逻辑
  - `fish/context.fish` — 终端上下文捕获
  - `fish/bindings.fish` — 键绑定
  - `fish/theme.fish` — 右提示栏（`fish_right_prompt`）
  - `fish/actions/` — 各命令实现

  *Rationale*: 与 ZSH 插件保持平行的目录结构，便于维护和对照。

- [ ] **1.2 实现 Fish 配置模块 (`fish/config.fish`)**  
  将 ZSH 的 `lib/config.zsh` 中所有变量转换为 Fish 语法：
  - `set -g _FORGE_BIN` (默认 forge)
  - `set -g _FORGE_CONVERSATION_ID`
  - `set -g _FORGE_ACTIVE_AGENT`
  - `set -g _FORGE_SESSION_MODEL` / `_FORGE_SESSION_PROVIDER` / `_FORGE_SESSION_REASONING_EFFORT`
  - `set -g _FORGE_TERM` / `_FORGE_TERM_MAX_COMMANDS` / `_FORGE_TERM_OSC133`
  - `set -g _FORGE_TERM_COMMANDS` / `_FORGE_TERM_EXIT_CODES` / `_FORGE_TERM_TIMESTAMPS`（列表变量）
  
  *Rationale*: Fish 使用 `set` 而非 `typeset`，列表是空格分隔的字符串。

- [ ] **1.3 实现 Fish 工具函数 (`fish/helpers.fish`)**  
  移植 `_forge_exec`、`_forge_exec_interactive`、`_forge_fzf`、`_forge_log`、`_forge_reset`、`_forge_get_commands`、`_forge_start_background_sync`、`_forge_start_background_update`。
  
  关键差异：
  - Fish 没有 `$()` 子shell 中继承 local 变量的问题，`set -x` 导出环境变量
  - Fish 的数组拼接用 `set -a` 或直接列表拼接
  - `local -x` → `set -lx`
  - 后台任务用 `&` 但 Fish 中不需要 `&!`（Fish 默认不通知后台任务）
  
  *Rationale*: 核心执行逻辑，所有命令动作都依赖这些函数。

- [ ] **1.4 实现 Fish 命令调度器 (`fish/dispatcher.fish`)**  
  Fish 没有 ZLE widget 系统，需要用不同的方式拦截 `:` 前缀命令。
  
  **推荐方案：使用 `fish_command_not_found` handler**
  - Fish 在命令未找到时会调用 `fish_command_not_found` 函数
  - 但 `:` 本身是 Fish 的 no-op 命令，不会被触发
  
  **实际方案：使用 `bind` 自定义 accept-line**
  - 创建 `__forge_accept_line` 函数
  - 绑定 Enter 键到该函数
  - 函数内检查 `commandline` 是否以 `:` 开头
  - 如果是，解析命令并路由到对应的 action handler
  - 如果不是，调用 `commandline -f execute`
  
  ```fish
  function __forge_accept_line
      set -l cmd (commandline)
      if string match -q ':*' -- $cmd
          # 解析并路由 :command
          __forge_dispatch $cmd
          commandline -r ''
          commandline -f repaint
      else
          commandline -f execute
      end
  end
  bind \r __forge_accept_line
  ```
  
  *Rationale*: Fish 的 `bind` + `commandline` 提供了与 ZSH ZLE 等价的能力，这是实现 `:` 前缀系统的核心。

- [ ] **1.5 实现 Fish 命令路由 (`fish/dispatcher.fish` 内)**  
  移植 ZSH dispatcher 中的正则解析和 case 路由：
  - `string match -r '^:([a-zA-Z][a-zA-Z0-9_-]*)( (.*))?$'` 解析 `:command args`
  - `string match -r '^: (.*)$'` 解析 `: prompt text`
  - 将所有 case 分支转换为 Fish `switch` 语句
  - 调用对应的 `__forge_action_*` 函数
  
  *Rationale*: 需要完整支持所有 ZSH 插件支持的命令（new/info/dump/compact/retry/agent/conversation/commit/suggest/edit 等）。

- [ ] **1.6 实现 Fish 动作处理器 (`fish/actions/*.fish`)**  
  逐个移植 `lib/actions/` 下的所有 action：
  - `core.fish` — `_forge_action_new`、`_forge_action_info`、`_forge_action_help`、`_forge_action_retry`、`_forge_action_copy`、`_forge_action_dump`、`_forge_action_compact`、`_forge_action_default`、`_forge_action_clone`、`_forge_action_rename`、`_forge_action_conversation_rename`、`_forge_action_sync`、`_forge_action_sync_init`、`_forge_action_sync_status`、`_forge_action_sync_info`、`_forge_action_skill`、`_forge_action_tools`
  - `config.fish` — `_forge_action_model`、`_forge_action_session_model`、`_forge_action_config_reload`、`_forge_action_reasoning_effort`、`_forge_action_config_reasoning_effort`、`_forge_action_commit_model`、`_forge_action_suggest_model`、`_forge_action_config`、`_forge_action_config_edit`
  - `conversation.fish` — `_forge_action_conversation`、`_forge_action_agent`
  - `git.fish` — `_forge_action_commit`、`_forge_action_commit_preview`
  - `editor.fish` — `_forge_action_editor`
  - `auth.fish` — `_forge_action_login`、`_forge_action_logout`
  - `provider.fish` — provider 相关动作
  
  关键 Fish 差异：
  - `echo` → `echo` 或 `printf`（Fish 的 `echo` 行为略有不同）
  - `read` 用于获取命令输出
  - `set -l output (command)` 替代 `local output=$(command)`
  
  *Rationale*: 功能对等的命令集合。

### Phase 2: Fish 补全与交互

- [ ] **2.1 实现 Fish Tab 补全 (`fish/completion.fish`)**  
  Fish 的补全系统与 ZSH 完全不同，使用 `complete` 命令：
  - 为 `:` 前缀命令注册补全
  - 使用 `complete -c forge -n 'string match -q \':*\' -- (commandline -ct)'` 条件补全
  - `@` 文件补全：使用 fzf pipe 方式，`forge list files --porcelain | fzf ...`
  - `:` 命令补全：使用 `forge list commands --porcelain | fzf ...`
  
  **替代方案（更推荐）**：自定义 Tab 绑定
  - 绑定 Tab 到自定义函数 `__forge_tab_complete`
  - 函数内检查当前光标位置，决定是文件补全还是命令补全
  - 这更接近 ZSH 插件的行为
  
  *Rationale*: Fish 的 `complete` 系统是为命令参数设计的，不太适合 `:` 前缀这种非标准语法。自定义绑定更灵活。

- [ ] **2.2 实现 Fish 右提示栏 (`fish/theme.fish`)**  
  创建 `fish_right_prompt` 函数：
  ```fish
  function fish_right_prompt
      set -l forge_bin "$_FORGE_BIN"
      if test -z "$forge_bin"
          set forge_bin forge
      end
      # 传递会话变量
      set -lx FORGE_SESSION__MODEL_ID "$_FORGE_SESSION_MODEL"
      set -lx FORGE_SESSION__PROVIDER_ID "$_FORGE_SESSION_PROVIDER"
      set -lx FORGE_REASONING__EFFORT "$_FORGE_SESSION_REASONING_EFFORT"
      set -lx _FORGE_CONVERSATION_ID "$_FORGE_CONVERSATION_ID"
      set -lx _FORGE_ACTIVE_AGENT "$_FORGE_ACTIVE_AGENT"
      $forge_bin zsh rprompt 2>/dev/null
  end
  ```
  
  *Rationale*: 复用 Rust 端的 `zsh rprompt` 命令（它生成的是 ANSI 着色字符串，与 shell 无关）。只需在 Fish 的 `fish_right_prompt` 中调用即可。

- [ ] **2.3 实现 Fish 终端上下文捕获 (`fish/context.fish`)**  
  Fish 使用事件系统而非 hook 数组：
  ```fish
  function __forge_preexec --on-event fish_preexec
      # 记录命令和时间戳
  end
  
  function __forge_precmd --on-event fish_postexec
      # 记录退出码
      # 发射 OSC 133 标记
  end
  ```
  
  注意：Fish 4.0+ 使用 `fish_preexec` / `fish_postexec` 事件。旧版本使用通用事件名。
  
  *Rationale*: Fish 的事件系统比 ZSH 的 `preexec_functions` 更简洁，但语义等价。

- [ ] **2.4 实现 Fish 键绑定 (`fish/bindings.fish`)**  
  - 绑定 Enter（`\r`）到 `__forge_accept_line`
  - 绑定 Tab（`\t`）到 `__forge_tab_complete`
  - Bracketed paste：Fish 内置支持，但需要自定义处理函数来包装 `@[path]`
    - Fish 的 bracketed paste 通过 `fish_clipboard_paste` 函数处理
    - 可以覆写该函数来添加路径包装逻辑
  
  *Rationale*: Fish 的 bracketed paste 是内置的，但路径格式化需要额外处理。

### Phase 3: Rust 端 Fish 支持

- [ ] **3.1 创建 Fish Rust 模块 (`crates/forge_main/src/fish/`)**  
  创建 `mod.rs`、`plugin.rs`、`setup.rs`，对应 ZSH 模块结构：
  - `fish/mod.rs` — 模块入口
  - `fish/plugin.rs` — 生成 Fish 插件脚本（嵌入 `shell-plugin/fish/` 下的文件）
  - `fish/setup.rs` — 安装到 `~/.config/fish/config.fish`
  
  *Rationale*: Rust 端负责生成和安装 Fish 插件脚本。

- [ ] **3.2 添加 Fish CLI 子命令**  
  在 `crates/forge_main/src/cli.rs` 中：
  - 将 `ZshCommandGroup` 重命名为更通用的名称，或添加并行的 `FishCommandGroup`
  - 添加 `forge fish plugin`、`forge fish theme`、`forge fish setup`、`forge fish doctor` 子命令
  - `forge setup` 命令增加 `--shell fish` 选项
  
  *Rationale*: CLI 层面支持 Fish。

- [ ] **3.3 实现 Fish 插件生成 (`fish/plugin.rs`)**  
  类似 `zsh/plugin.rs` 的 `generate_zsh_plugin()`：
  - 使用 `include_dir!` 嵌入 `shell-plugin/fish/` 下的所有文件
  - 合并为一个 Fish 脚本输出
  - 末尾设置 `_FORGE_PLUGIN_LOADED` 标记
  
  *Rationale*: 与 ZSH 插件生成逻辑对称。

- [ ] **3.4 实现 Fish 安装逻辑 (`fish/setup.rs`)**  
  类似 `setup_zsh_integration()`：
  - 定位 Fish 配置文件（`~/.config/fish/config.fish`，或 `$XDG_CONFIG_HOME/fish/config.fish`）
  - 使用标记（`# >>> forge initialize >>>` / `# <<< forge initialize <<<`）管理注入块
  - 添加 `eval (forge fish plugin)` 和 `source (forge fish theme)` 到配置文件
  - 创建备份
  
  *Rationale*: 安装流程需要 Fish 特定的路径和语法。

- [ ] **3.5 实现 Fish doctor 诊断**  
  创建 `shell-plugin/fish/doctor.fish`：
  - 检查 Fish 版本
  - 检查 fzf 是否安装
  - 检查插件是否加载
  - 检查 bat 是否安装
  
  *Rationale*: 与 ZSH doctor 对等。

### Phase 4: Bracketed Paste 路径格式化（Fish）

- [ ] **4.1 Fish bracketed paste 路径包装**  
  在 Fish 插件中实现与 ZSH `forge-bracketed-paste` 等价的功能：
  
  方案 A（推荐）：覆写 `fish_clipboard_paste`
  ```fish
  function fish_clipboard_paste
      # 调用原始粘贴
      commandline -i (pbpaste 2>/dev/null; or xclip -selection clipboard -o 2>/dev/null; or wl-paste 2>/dev/null)
      # 如果是 : 命令，格式化路径
      set -l buf (commandline)
      if string match -q ':*' -- $buf
          set -l formatted ($_FORGE_BIN zsh format --buffer $buf)
          if test -n "$formatted" -a "$formatted" != "$buf"
              commandline -r $formatted
          end
      end
      commandline -f repaint
  end
  ```
  
  方案 B：使用 Fish 的 `fish_paste` 事件
  
  *Rationale*: 复用 Rust 端的 `zsh format` 命令（它是 shell 无关的路径格式化逻辑）。

### Phase 5: 粘贴图片功能

- [ ] **5.1 添加 `:paste-image` 命令**  
  在 ZSH 和 Fish 插件中都添加 `:paste-image`（别名 `:pi`）命令：
  - 调用系统剪贴板工具读取图片数据
  - 支持的平台检测：
    - macOS: `osascript` 读取剪贴板图片，或 `pngpaste` 工具
    - Linux X11: `xclip -selection clipboard -t image/png -o`
    - Linux Wayland: `wl-paste --type image/png`
  - 将图片保存到临时文件（`/tmp/forge-paste-{timestamp}.png`）
  - 在命令行插入 `@[tempfile.png]`
  
  *Rationale*: 这是最通用的跨终端、跨平台方案。

- [ ] **5.2 实现 Rust 端图片粘贴服务**  
  在 `crates/forge_services/src/` 中添加 `clipboard_image.rs`：
  - 定义 `ClipboardImageService` trait
  - 实现 `read_clipboard_image()` 方法
  - 平台检测和工具选择
  - 图片格式检测（PNG/JPEG/WebP/GIF）
  - 临时文件保存
  
  *Rationale*: Rust 端处理图片比 shell 脚本更可靠，能正确处理二进制数据和格式检测。

- [ ] **5.3 添加 `forge clipboard paste-image` CLI 子命令**  
  在 CLI 中添加 `forge clipboard paste-image` 命令：
  - 读取剪贴板图片
  - 保存为临时文件
  - 输出文件路径（供 shell 插件使用）
  
  *Rationale*: Shell 插件调用 `forge clipboard paste-image` 获取路径，然后插入到命令行。

- [ ] **5.4 TUI 模式（reedline）中的图片粘贴支持**  
  在 `crates/forge_main/src/editor.rs` 的 `ForgeEditMode::parse_event` 中：
  - 检测 `Event::Paste` 事件是否包含图片数据（通常是 base64 或特殊标记）
  - 目前 reedline/crossterm 的 `Event::Paste` 只支持文本，不支持二进制图片
  - **短期方案**：在 TUI 中添加 Ctrl+V / Cmd+V 快捷键，触发剪贴板图片读取
  - **长期方案**：等待终端协议标准化剪贴板图片传输
  
  *Rationale*: TUI 模式的图片粘贴受限于终端协议，需要渐进式实现。

- [ ] **5.5 在 ZSH 插件中添加 `:paste-image` 命令**  
  在 `shell-plugin/lib/actions/core.zsh` 中添加 `_forge_action_paste_image`：
  ```zsh
  function _forge_action_paste_image() {
      local temp_path=$($_FORGE_BIN clipboard paste-image)
      if [[ -n "$temp_path" ]]; then
          BUFFER=":@[${temp_path}] "
          CURSOR=${#BUFFER}
          zle reset-prompt
      fi
  }
  ```
  在 dispatcher 的 case 中添加 `paste-image|pi)` 分支。
  
  *Rationale*: ZSH 端对等支持。

- [ ] **5.6 在 Fish 插件中添加 `:paste-image` 命令**  
  在 Fish dispatcher 中添加 `paste-image` / `pi` case：
  ```fish
  function __forge_action_paste_image
      set -l temp_path ($_FORGE_BIN clipboard paste-image)
      if test -n "$temp_path"
          commandline -r ":@[$temp_path] "
      end
      commandline -f repaint
  end
  ```
  
  *Rationale*: Fish 端对等支持。

### Phase 6: 测试与文档

- [ ] **6.1 Fish 插件集成测试**  
  - 测试所有 `:` 命令在 Fish 中正常工作
  - 测试 `@[file]` 补全
  - 测试右提示栏显示
  - 测试 bracketed paste 路径格式化
  - 测试 `forge fish setup` 安装和卸载
  - 测试 `forge fish doctor` 诊断
  
  *Rationale*: 确保功能对等。

- [ ] **6.2 粘贴图片功能测试**  
  - 测试各平台剪贴板图片读取
  - 测试图片格式检测
  - 测试临时文件创建和清理
  - 测试 `@[image]` 附件发送到 AI
  
  *Rationale*: 确保图片功能可靠。

- [ ] **6.3 更新 README 和文档**  
  - 在 README 中添加 Fish shell 支持说明
  - 更新安装说明，包含 Fish 安装步骤
  - 添加 `:paste-image` 命令文档
  - 更新 `forge setup` 帮助文本，说明 `--shell` 选项

## Verification Criteria

- [ ] **Fish 基础功能**：在 Fish shell 中输入 `: hello world` 能正确发送 prompt 到 Forge
- [ ] **Fish 命令系统**：所有 ZSH 支持的 `:` 命令（`:new`、`:commit`、`:suggest`、`:agent` 等）在 Fish 中都能正常工作
- [ ] **Fish Tab 补全**：按 Tab 能触发 `@` 文件补全和 `:` 命令补全
- [ ] **Fish 右提示栏**：`fish_right_prompt` 显示 agent/model/token 信息
- [ ] **Fish 终端上下文**：preexec/postexec hooks 正确捕获命令历史
- [ ] **Fish 安装**：`forge fish setup` 能正确修改 `config.fish`
- [ ] **粘贴图片**：`:paste-image` 能从系统剪贴板读取图片并插入为 `@[path]`
- [ ] **粘贴图片 - 附件**：包含 `@[image.png]` 的 prompt 能正确将图片作为附件发送给 AI

## Potential Risks and Mitigations

1. **Fish 语法高亮不可编程**
   - Fish 的语法高亮是内置的，不支持像 ZSH 那样通过 `ZSH_HIGHLIGHT_PATTERNS` 自定义
   - Mitigation: `:` 前缀命令在 Fish 中会被识别为未知命令（红色），这是 Fish 的默认行为。可以通过 Fish 的 `fish_color_error` 调整，但这会影响所有错误。最实际的方案是接受这个限制，或考虑使用 Fish 的 abbreviations 系统来改善体验。

2. **Fish 的 `:` 命令冲突**
   - Fish 中 `:` 是内置的 no-op 命令，`: something` 不会触发 command_not_found
   - Mitigation: 使用自定义 Enter 键绑定拦截，在命令执行前检查是否以 `:` 开头。这比 ZSH 的 ZLE widget 方式更直接。

3. **剪贴板图片读取的平台差异**
   - 不同平台（macOS/Linux X11/Linux Wayland/Windows）的剪贴板工具不同
   - Mitigation: 实现平台检测链，依次尝试可用的工具。提供清晰的错误信息指导用户安装所需工具。

4. **Fish 版本兼容性**
   - Fish 3.x 和 4.x 的事件系统有差异（`fish_preexec` vs `fish_postexec`）
   - Mitigation: 检测 Fish 版本，使用兼容的 API。优先支持 Fish 3.6+（当前稳定版）。

5. **Bracketed paste 路径格式化在 Fish 中的实现差异**
   - Fish 的 bracketed paste 处理是内置的，不如 ZSH 那样容易自定义
   - Mitigation: 通过覆写 `fish_clipboard_paste` 函数或在 paste 后检查 buffer 来实现。

## Alternative Approaches

1. **Fish 插件使用 Fisher/Oh-My-Fish 分发**：
   - 除了 `forge fish setup` 直接修改 config.fish 外，还可以发布为 Fisher 插件
   - 优点：更符合 Fish 社区的安装习惯
   - 缺点：需要维护额外的包文件

2. **使用 `command --query` 替代自定义 Enter 绑定**：
   - 利用 Fish 的 `command-not-found` handler
   - 缺点：`:` 是 Fish 内置命令，不会触发 command-not-found
   - 结论：不可行

3. **图片粘贴使用 Kitty 专用协议**：
   - 利用 Kitty 的文件传输协议直接获取剪贴板图片
   - 优点：更高效，不需要外部工具
   - 缺点：仅限 Kitty 终端
   - 结论：可作为优化项，但不作为主要方案

4. **图片粘贴使用 Screenshot 工具**：
   - 添加 `:screenshot` 命令，调用系统截图工具
   - 优点：不需要预先复制到剪贴板
   - 缺点：平台差异更大
   - 结论：可作为后续增强功能
