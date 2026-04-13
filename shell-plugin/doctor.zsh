#!/usr/bin/env zsh

# ZSH Doctor - Diagnostic tool for Forge shell environment
# Checks for common configuration issues and environment setup

# Source user's .zshrc to get their environment (suppress errors from non-interactive mode)
if [[ -f "${ZDOTDIR:-$HOME}/.zshrc" ]]; then
    source "${ZDOTDIR:-$HOME}/.zshrc" 2>/dev/null
fi

# ANSI codes
local RESET='\033[0m'
local _BOLD='\033[1m'
local _DIM='\033[2m'
local _GREEN='\033[0;32m'
local _RED='\033[0;31m'
local _YELLOW='\033[0;33m'
local _CYAN='\033[0;36m'

# Text formatting helpers - auto-reset
function bold() { echo "${_BOLD}${1}${RESET}"; }
function dim() { echo "${_DIM}${1}${RESET}"; }
function green() { echo "${_GREEN}${1}${RESET}"; }
function red() { echo "${_RED}${1}${RESET}"; }
function yellow() { echo "${_YELLOW}${1}${RESET}"; }
function cyan() { echo "${_CYAN}${1}${RESET}"; }

# Simple ASCII symbols
local PASS="[OK]"
local FAIL="[ERROR]"
local WARN="[WARN]"

# Counters
local passed=0
local failed=0
local warnings=0

# Helper function to print section headers
function print_section() {
    echo ""
    echo "$(bold "$1")"
}

# Helper function to print results
function print_result() {
    local result_status=$1
    local message=$2
    local detail=$3
    
    case $result_status in
        pass)
            echo "  $(green "${PASS}") ${message}"
            ((passed++))
            ;;
        fail)
            echo "  $(red "${FAIL}") ${message}"
            [[ -n "$detail" ]] && echo "  $(dim "· ${detail}")"
            ((failed++))
            ;;
        warn)
            echo "  $(yellow "${WARN}") ${message}"
            [[ -n "$detail" ]] && echo "  $(dim "· ${detail}")"
            ((warnings++))
            ;;
        info)
            echo "  $(dim "· ${message}")"
            ;;
        code)
            echo "  $(dim "· ${message}")"
            ;;
        instruction)
            echo "  $(dim "· ${message}")"
            ;;
    esac
}

echo "$(bold "FORGE ENVIRONMENT DIAGNOSTICS")"

# 1. Check ZSH version
print_section "Shell Environment"
local zsh_version="${ZSH_VERSION}"
if [[ -n "$zsh_version" ]]; then
    local major=$(echo $zsh_version | cut -d. -f1)
    local minor=$(echo $zsh_version | cut -d. -f2)
    if [[ $major -ge 5 ]] && [[ $minor -ge 0 ]]; then
        print_result pass "zsh: ${zsh_version}"
    else
        print_result warn "zsh: ${zsh_version}" "Recommended: 5.0+"
    fi
else
    print_result fail "Unable to detect ZSH version"
fi

# Check terminal information
if [[ -n "$TERM_PROGRAM" ]]; then
    if [[ -n "$TERM_PROGRAM_VERSION" ]]; then
        print_result pass "Terminal: ${TERM_PROGRAM} ${TERM_PROGRAM_VERSION}"
    else
        print_result pass "Terminal: ${TERM_PROGRAM}"
    fi
elif [[ -n "$TERM" ]]; then
    print_result pass "Terminal: ${TERM}"
else
    print_result info "Terminal: unknown"
fi


# Check if Oh My Zsh is installed
if [[ -n "$ZSH" ]] && [[ -d "$ZSH" ]]; then
    local omz_version=""
    # Try to get OMZ version from version file if it exists
    if [[ -f "$ZSH/.git/refs/heads/master" ]]; then
        omz_version=$(cd "$ZSH" && git describe --tags 2>/dev/null || echo "custom")
    fi
    
    if [[ -n "$omz_version" ]]; then
        print_result pass "Oh My Zsh: ${omz_version}"
    else
        print_result pass "Oh My Zsh: installed"
    fi
    print_result info "${ZSH}"
else
    print_result warn "Oh My Zsh not found" "Install: sh -c \"\$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)\""
fi

# 2. Check if forge is installed and in PATH
print_section "Forge Installation"

# Check if forge is in PATH
if command -v forge &> /dev/null; then
    local forge_path=$(command -v forge)
    
    # Get forge version and extract just the version number
    local forge_version=$(forge --version 2>&1 | head -n1 | awk '{print $2}')
    if [[ -n "$forge_version" ]]; then
        print_result pass "forge: ${forge_version}"
        print_result info "${forge_path}"
    else
        print_result pass "forge: installed"
        print_result info "${forge_path}"
    fi
else
    print_result fail "Forge binary not found in PATH" "Installation: curl -fsSL https://forgecode.dev/cli | sh"
fi

# 3. Check shell plugin
print_section "Plugin"

# Check if forge plugin is loaded by checking environment variable
if [[ -n "$_FORGE_PLUGIN_LOADED" ]]; then
    print_result pass "Forge plugin loaded"
else
    print_result fail "Forge plugin not loaded"
    print_result instruction "Add to your ~/.zshrc:"
    print_result code "eval \"\$(forge zsh plugin)\""
    print_result instruction "Or run: forge zsh setup"
fi


# Check plugin loading order in .zshrc
local zshrc_file="${ZDOTDIR:-$HOME}/.zshrc"
if [[ -f "$zshrc_file" ]] && [[ -n "$_FORGE_PLUGIN_LOADED" ]]; then
    # Extract line numbers for plugin declarations and forge plugin eval
    local plugins_line=$(grep -n "^[[:space:]]*plugins=(" "$zshrc_file" 2>/dev/null | head -n1 | cut -d: -f1)
    local forge_plugin_line=$(grep -n "eval.*forge.*zsh plugin" "$zshrc_file" 2>/dev/null | head -n1 | cut -d: -f1)

    if [[ -n "$plugins_line" ]] && [[ -n "$forge_plugin_line" ]]; then
        if [[ $forge_plugin_line -lt $plugins_line ]]; then
            print_result fail "Plugin loading order incorrect"
            print_result instruction "Forge plugin (line ${forge_plugin_line}) should be loaded AFTER plugins=() (line ${plugins_line})"
            print_result instruction "Move the forge plugin eval statement after the plugins=() array in ~/.zshrc"
        else
            print_result pass "Plugin loading order correct"
        fi
    elif [[ -n "$forge_plugin_line" ]] && [[ -z "$plugins_line" ]]; then
        # Forge plugin found but no plugins=() array - check for individual plugin sources
        local has_other_plugins=false
        if grep -q "source.*zsh-autosuggestions" "$zshrc_file" 2>/dev/null || \
           grep -q "source.*zsh-syntax-highlighting" "$zshrc_file" 2>/dev/null; then
            has_other_plugins=true
        fi
        
        if [[ "$has_other_plugins" == "true" ]]; then
            print_result warn "Manual plugin loading detected"
            print_result info "Ensure forge plugin is sourced AFTER zsh-autosuggestions and zsh-syntax-highlighting"
        fi
    fi
fi

# 4. Check ZSH theme RPROMPT
print_section "FORGE RIGHT PROMPT"

# Check if forge theme is loaded by checking environment variable
if [[ -n "$_FORGE_THEME_LOADED" ]]; then
    print_result pass "Forge theme loaded"
elif (( $+functions[p10k] )); then
    print_result info "Powerlevel10k detected (not using Forge theme)"
elif [[ -n "$ZSH_THEME" ]]; then
    print_result warn "Using theme: ${ZSH_THEME}"
    print_result instruction "To use Forge theme, add to ~/.zshrc:"
    print_result code "eval \"\$(forge zsh theme)\""
else
    print_result warn "No theme loaded"
    print_result instruction "To use Forge theme, add to ~/.zshrc:"
    print_result code "eval \"\$(forge zsh theme)\""
fi

# Helper function to compare versions
# Returns 0 if version1 >= version2, 1 otherwise
function version_gte() {
    local version1=$1
    local version2=$2
    
    # Remove 'v' prefix if present
    version1=${version1#v}
    version2=${version2#v}
    
    # Split versions into arrays
    local -a ver1_parts=(${(s:.:)version1})
    local -a ver2_parts=(${(s:.:)version2})
    
    # Compare each part
    for i in {1..3}; do
        local v1=${ver1_parts[$i]:-0}
        local v2=${ver2_parts[$i]:-0}
        
        # Remove any non-numeric suffix (e.g., "0-rc1" -> "0")
        v1=${v1%%[^0-9]*}
        v2=${v2%%[^0-9]*}
        
        if [[ $v1 -gt $v2 ]]; then
            return 0
        elif [[ $v1 -lt $v2 ]]; then
            return 1
        fi
    done
    
    return 0  # versions are equal
}

# 5. Check dependencies
print_section "Dependencies"

# Check for fzf - required for interactive selection
if command -v fzf &> /dev/null; then
    local fzf_version=$(fzf --version 2>&1 | head -n1 | awk '{print $1}')
    if [[ -n "$fzf_version" ]]; then
        if version_gte "$fzf_version" "0.36.0"; then
            print_result pass "fzf: ${fzf_version}"
        else
            print_result fail "fzf: ${fzf_version}" "Version 0.36.0 or higher required. Update: https://github.com/junegunn/fzf#installation"
        fi
    else
        print_result pass "fzf: installed"
    fi
else
    print_result fail "fzf not found" "Required for interactive features. See installation: https://github.com/junegunn/fzf#installation"
fi

# Check for fd/fdfind - used for file discovery
if command -v fd &> /dev/null; then
    local fd_version=$(fd --version 2>&1 | awk '{print $2}')
    if [[ -n "$fd_version" ]]; then
        if version_gte "$fd_version" "10.0.0"; then
            print_result pass "fd: ${fd_version}"
        else
            print_result fail "fd: ${fd_version}" "Version 10.0.0 or higher required. Update: https://github.com/sharkdp/fd#installation"
        fi
    else
        print_result pass "fd: installed"
    fi
elif command -v fdfind &> /dev/null; then
    local fd_version=$(fdfind --version 2>&1 | awk '{print $2}')
    if [[ -n "$fd_version" ]]; then
        if version_gte "$fd_version" "10.0.0"; then
            print_result pass "fdfind: ${fd_version}"
        else
            print_result fail "fdfind: ${fd_version}" "Version 10.0.0 or higher required. Update: https://github.com/sharkdp/fd#installation"
        fi
    else
        print_result pass "fdfind: installed"
    fi
else
    print_result warn "fd/fdfind not found" "Enhanced file discovery. See installation: https://github.com/sharkdp/fd#installation"
fi

# Check for bat - used for syntax highlighting
if command -v bat &> /dev/null; then
    local bat_version=$(bat --version 2>&1 | awk '{print $2}')
    if [[ -n "$bat_version" ]]; then
        if version_gte "$bat_version" "0.20.0"; then
            print_result pass "bat: ${bat_version}"
        else
            print_result fail "bat: ${bat_version}" "Version 0.20.0 or higher required. Update: https://github.com/sharkdp/bat#installation"
        fi
    else
        print_result pass "bat: installed"
    fi
else
    print_result warn "bat not found" "Enhanced preview. See installation: https://github.com/sharkdp/bat#installation"
fi

# 6. Check required ZSH plugins
print_section "Required Plugins"

# Check for zsh-autosuggestions
if [[ " ${plugins[*]} " =~ " zsh-autosuggestions " ]] || \
   [[ -n "$fpath[(r)*zsh-autosuggestions*]" ]] || \
   (( $+functions[_zsh_autosuggest_accept] )); then
    print_result pass "zsh-autosuggestions loaded"
else
    print_result warn "zsh-autosuggestions not found"
    print_result info "Install plugin and add to plugins=() in .zshrc"
    print_result info "Installation guide: https://github.com/zsh-users/zsh-autosuggestions/blob/master/INSTALL.md"
fi

# Check for zsh-syntax-highlighting
if [[ " ${plugins[*]} " =~ " zsh-syntax-highlighting " ]] || \
   [[ -n "$fpath[(r)*zsh-syntax-highlighting*]" ]] || \
   (( $+functions[_zsh_highlight] )); then
    print_result pass "zsh-syntax-highlighting loaded"
else
    print_result warn "zsh-syntax-highlighting not found"
    print_result info "Install plugin and add to plugins=() in .zshrc"
    print_result info "Installation guide: https://github.com/zsh-users/zsh-syntax-highlighting/blob/master/INSTALL.md"
fi

# 7. Check system configuration
print_section "System"

# Check editor configuration (FORGE_EDITOR takes precedence over EDITOR)
if [[ -n "$FORGE_EDITOR" ]]; then
    print_result pass "FORGE_EDITOR: ${FORGE_EDITOR}"
    if [[ -n "$EDITOR" ]]; then
        print_result info "EDITOR also set: ${EDITOR} (ignored)"
    fi
elif [[ -n "$EDITOR" ]]; then
    print_result pass "EDITOR: ${EDITOR}"
    print_result info "TIP: Set FORGE_EDITOR for forge-specific editor"
else
    print_result warn "No editor configured" "export EDITOR=vim or export FORGE_EDITOR=vim"
fi

# Check PATH for common issues
if [[ "$PATH" == *"/usr/local/bin"* ]] || [[ "$PATH" == *"/usr/bin"* ]]; then
    print_result pass "PATH: configured"
else
    print_result warn "PATH may need common directories" "Ensure /usr/local/bin or /usr/bin is in PATH"
fi

# 7. Check keyboard configuration (Alt/Option key as Meta)
print_section "Keyboard Configuration"

local platform=$(uname)
local meta_key_ok=false
local check_performed=false

if [[ "$platform" == "Darwin" ]]; then
    # macOS checks
    if [[ "$TERM_PROGRAM" == "vscode" ]]; then
        check_performed=true
        # Check VS Code settings
        local vscode_settings="${HOME}/Library/Application Support/Code/User/settings.json"
        if [[ -f "$vscode_settings" ]]; then
            if grep -q '"terminal.integrated.macOptionIsMeta"[[:space:]]*:[[:space:]]*true' "$vscode_settings" 2>/dev/null; then
                print_result pass "VS Code: Option key configured as Meta"
                meta_key_ok=true
            else
                print_result warn "VS Code: Option key NOT configured as Meta"
                print_result instruction "Option+F and Option+B shortcuts won't work for word navigation"
                print_result instruction "Add to VS Code settings.json:"
                print_result code '"terminal.integrated.macOptionIsMeta": true'
                print_result instruction "Then reload VS Code: Cmd+Shift+P → Reload Window"
            fi
        else
            print_result warn "VS Code settings file not found"
            print_result info "Expected: ${vscode_settings}"
        fi
    elif [[ "$TERM_PROGRAM" == "iTerm.app" ]]; then
        check_performed=true
        # Check iTerm2 preferences
        local iterm_prefs="${HOME}/Library/Preferences/com.googlecode.iterm2.plist"
        if [[ -f "$iterm_prefs" ]]; then
            # Check if either Left or Right Option key is set to Esc+ (value 2)
            local option_setting=$(defaults read com.googlecode.iterm2 2>/dev/null | grep -E '"(Left |Right )?Option Key Sends"' | grep -o '[0-9]' | head -1)
            if [[ "$option_setting" == "2" ]]; then
                print_result pass "iTerm2: Option key configured as Esc+"
                meta_key_ok=true
            else
                print_result warn "iTerm2: Option key NOT configured as Esc+"
                print_result instruction "Option+F and Option+B shortcuts won't work for word navigation"
                print_result instruction "Configure in iTerm2:"
                print_result info "Preferences → Profiles → Keys → Left/Right Option Key → Esc+"
            fi
        else
            print_result warn "iTerm2 preferences not found"
            print_result info "Expected: ${iterm_prefs}"
        fi
    elif [[ "$TERM_PROGRAM" == "Apple_Terminal" ]]; then
        check_performed=true
        # Check Terminal.app preferences
        local terminal_prefs="${HOME}/Library/Preferences/com.apple.Terminal.plist"
        if [[ -f "$terminal_prefs" ]]; then
            local use_option=$(defaults read com.apple.Terminal 2>/dev/null | grep -E 'useOptionAsMetaKey' | grep -o '[0-9]' | head -1)
            if [[ "$use_option" == "1" ]]; then
                print_result pass "Terminal.app: Option key configured as Meta"
                meta_key_ok=true
            else
                print_result warn "Terminal.app: Option key NOT configured as Meta"
                print_result instruction "Option+F and Option+B shortcuts won't work for word navigation"
                print_result instruction "Configure in Terminal.app:"
                print_result info "Preferences → Profiles → Keyboard → ✓ Use Option as Meta key"
            fi
        else
            print_result warn "Terminal.app preferences not found"
            print_result info "Expected: ${terminal_prefs}"
        fi
    fi
    
    # If no specific terminal detected, provide general guidance for macOS
    if [[ "$check_performed" == "false" ]]; then
        print_result info "Terminal: ${TERM_PROGRAM:-unknown}"
        print_result info "For Option key shortcuts (word navigation) to work:"
        print_result info "• VS Code: Settings → terminal.integrated.macOptionIsMeta → true"
        print_result info "• iTerm2: Preferences → Profiles → Keys → Option Key → Esc+"
        print_result info "• Terminal.app: Preferences → Profiles → Keyboard → Use Option as Meta"
        print_result info "Run 'forge zsh keyboard' for detailed keyboard shortcuts"
    fi
    
elif [[ "$platform" == "Linux" ]]; then
    # Linux checks
    if [[ "$TERM_PROGRAM" == "vscode" ]]; then
        check_performed=true
        # Check VS Code settings on Linux
        local vscode_settings="${HOME}/.config/Code/User/settings.json"
        if [[ -f "$vscode_settings" ]]; then
            # On Linux, check for sendAltAsMetaKey (deprecated but still works) or macOptionIsMeta
            if grep -q '"terminal.integrated.sendAltAsMetaKey"[[:space:]]*:[[:space:]]*true' "$vscode_settings" 2>/dev/null || \
               grep -q '"terminal.integrated.macOptionIsMeta"[[:space:]]*:[[:space:]]*true' "$vscode_settings" 2>/dev/null; then
                print_result pass "VS Code: Alt key configured as Meta"
                meta_key_ok=true
            else
                print_result warn "VS Code: Alt key NOT configured as Meta"
                print_result instruction "Alt+F and Alt+B shortcuts won't work for word navigation"
                print_result instruction "Add to VS Code settings.json:"
                print_result code '"terminal.integrated.sendAltAsMetaKey": true'
                print_result instruction "Then reload VS Code: Ctrl+Shift+P → Reload Window"
            fi
        else
            print_result warn "VS Code settings file not found"
            print_result info "Expected: ${vscode_settings}"
        fi
    elif [[ -n "$GNOME_TERMINAL_SERVICE" ]] || [[ "$COLORTERM" == "gnome-terminal" ]]; then
        check_performed=true
        # GNOME Terminal check
        local profile_id=$(gsettings get org.gnome.Terminal.ProfilesList default 2>/dev/null | tr -d "'")
        if [[ -n "$profile_id" ]]; then
            local alt_sends_escape=$(gsettings get org.gnome.Terminal.Legacy.Profile:/org/gnome/terminal/legacy/profiles:/:${profile_id}/ use-theme-colors 2>/dev/null)
            # GNOME Terminal doesn't have a simple setting check - most distributions enable Alt by default
            print_result pass "GNOME Terminal: Alt key typically works by default"
            print_result info "If Alt+F/B don't work, check: Preferences → Profile → Keyboard"
            meta_key_ok=true
        else
            print_result info "GNOME Terminal detected"
            print_result info "Alt key typically works by default for word navigation"
        fi
    elif [[ "$COLORTERM" == "truecolor" ]] && command -v konsole &> /dev/null; then
        check_performed=true
        # Konsole (KDE) - Alt typically works by default
        print_result pass "Konsole: Alt key typically works by default"
        print_result info "If Alt+F/B don't work, check: Settings → Edit Profile → Keyboard"
        meta_key_ok=true
    elif [[ -n "$ALACRITTY_SOCKET" ]] || [[ "$TERM" == "alacritty" ]]; then
        check_performed=true
        # Alacritty check - look for config file
        local alacritty_config="${HOME}/.config/alacritty/alacritty.yml"
        local alacritty_config_toml="${HOME}/.config/alacritty/alacritty.toml"
        
        if [[ -f "$alacritty_config" ]] || [[ -f "$alacritty_config_toml" ]]; then
            print_result pass "Alacritty: Alt key typically works by default"
            print_result info "If Alt+F/B don't work, ensure no conflicting key bindings"
            meta_key_ok=true
        else
            print_result pass "Alacritty: Alt key typically works by default"
            print_result info "Config: ${alacritty_config} or ${alacritty_config_toml}"
        fi
    elif [[ "$TERM" == "xterm" ]] || [[ "$TERM" == "xterm-256color" ]]; then
        check_performed=true
        # xterm - check .Xresources
        local xresources="${HOME}/.Xresources"
        if [[ -f "$xresources" ]]; then
            if grep -q "XTerm\*metaSendsEscape:[[:space:]]*true" "$xresources" 2>/dev/null || \
               grep -q "XTerm\*eightBitInput:[[:space:]]*false" "$xresources" 2>/dev/null; then
                print_result pass "xterm: Meta key configured"
                meta_key_ok=true
            else
                print_result warn "xterm: Meta key may not be configured"
                print_result instruction "Add to ~/.Xresources:"
                print_result code "XTerm*metaSendsEscape: true"
                print_result instruction "Then reload: xrdb ~/.Xresources"
            fi
        else
            print_result info "xterm detected"
            print_result info "To enable Alt as Meta, add to ~/.Xresources:"
            print_result info "XTerm*metaSendsEscape: true"
        fi
    fi
    
    # If no specific terminal detected, provide general guidance for Linux
    if [[ "$check_performed" == "false" ]]; then
        print_result info "Terminal: ${TERM_PROGRAM:-$TERM}"
        print_result info "For Alt key shortcuts (word navigation) to work:"
        print_result info "• VS Code: Settings → terminal.integrated.sendAltAsMetaKey → true"
        print_result info "• GNOME Terminal: Usually works by default"
        print_result info "• Konsole: Usually works by default"
        print_result info "• xterm: Add 'XTerm*metaSendsEscape: true' to ~/.Xresources"
        print_result info "Run 'forge zsh keyboard' for detailed keyboard shortcuts"
    fi
else
    # Other platforms (BSD, etc.)
    print_result info "Keyboard check: Platform ${platform} - manual verification needed"
    print_result info "Ensure Alt/Meta key is configured for word navigation shortcuts"
fi

# 8. Check font and Nerd Font support
print_section "Nerd Font"

# Check if Nerd Font is enabled via environment variables
if [[ -n "$NERD_FONT" ]]; then
    if [[ "$NERD_FONT" == "1" || "$NERD_FONT" == "true" ]]; then
        print_result pass "NERD_FONT: enabled"
    else
        print_result warn "NERD_FONT: disabled (${NERD_FONT})"
        print_result instruction "Enable Nerd Font by setting:"
        print_result code "export NERD_FONT=1"
    fi
elif [[ -n "$USE_NERD_FONT" ]]; then
    if [[ "$USE_NERD_FONT" == "1" || "$USE_NERD_FONT" == "true" ]]; then
        print_result pass "USE_NERD_FONT: enabled"
    else
        print_result warn "USE_NERD_FONT: disabled (${USE_NERD_FONT})"
        print_result instruction "Enable Nerd Font by setting:"
        print_result code "export NERD_FONT=1"
    fi
else
    print_result pass "Nerd Font: enabled (default)"
    print_result info "Forge will auto-detect based on terminal capabilities"
fi

# Show actual icons used in Forge theme for manual verification (skip if explicitly disabled)
local nerd_font_disabled=false
if [[ -n "$NERD_FONT" && "$NERD_FONT" != "1" && "$NERD_FONT" != "true" ]]; then
    nerd_font_disabled=true
elif [[ -n "$USE_NERD_FONT" && "$USE_NERD_FONT" != "1" && "$USE_NERD_FONT" != "true" ]]; then
    nerd_font_disabled=true
fi

if [[ "$nerd_font_disabled" == "false" ]]; then
    echo ""
    echo "$(yellow "Visual Check [Manual Verification Required]")"
echo "   $(bold "󱙺 FORGE 33.0k") $(cyan " tonic-1.0")"
    echo ""
    echo "   Forge uses Nerd Fonts to enrich cli experience, can you see all the icons clearly without any overlap?"
    echo "   If you see boxes (□) or question marks (?), install a Nerd Font from:"
    echo "   $(dim "https://www.nerdfonts.com/")"
    echo ""
fi

# Summary
echo ""

if [[ $failed -eq 0 && $warnings -eq 0 ]]; then
    echo "$(green "${PASS}") $(bold "All checks passed") $(dim "(${passed})")"
    exit 0
elif [[ $failed -eq 0 ]]; then
    echo "$(yellow "${WARN}") $(bold "${warnings} warnings") $(dim "(${passed} passed)")"
    exit 0
else
    echo "$(red "${FAIL}") $(bold "${failed} failed") $(dim "(${warnings} warnings, ${passed} passed)")"
    exit 1
fi
