# Forge Code - Fish Shell Plugin
# https://github.com/antinomyhq/forge
# Provides :command integration, right prompt, and key bindings

# ── Fisher lifecycle events ───────────────────────────────────────────────────
function _forge_install --on-event forge_install
    # Runs once after: fisher install
end

function _forge_update --on-event forge_update
    # Runs once after: fisher update
end

function _forge_uninstall --on-event forge_uninstall
    # Clean up key bindings
    bind --erase \r
    bind --erase \n
    bind --erase \t

    # Clean up global variables
    set --erase _FORGE_BIN
    set --erase _FORGE_MAX_COMMIT_DIFF
    set --erase _FORGE_DELIMITER
    set --erase _FORGE_PREVIEW_WINDOW
    set --erase _FORGE_CONVERSATION_ID
    set --erase _FORGE_PREVIOUS_CONVERSATION_ID
    set --erase _FORGE_ACTIVE_AGENT
    set --erase _FORGE_SESSION_MODEL
    set --erase _FORGE_SESSION_PROVIDER
    set --erase _FORGE_SESSION_REASONING_EFFORT
    set --erase _FORGE_COMMANDS
    set --erase _FORGE_FD_CMD
    set --erase _FORGE_CAT_CMD
    set --erase _FORGE_THEME_LOADED
    set --erase _FORGE_PLUGIN_LOADED

    # Clean up :command abbreviation
    abbr --erase _forge_cmd 2>/dev/null

    # Restore original right prompt if we wrapped it
    if functions -q _forge_original_fish_right_prompt
        functions -c _forge_original_fish_right_prompt fish_right_prompt
        functions -e _forge_original_fish_right_prompt
    end
end

# ── Guard: interactive shells only ────────────────────────────────────────────
if not status is-interactive
    return
end

# ── Global variables ──────────────────────────────────────────────────────────
set -g _FORGE_BIN (command -v forge 2>/dev/null; or echo forge)
set -g _FORGE_MAX_COMMIT_DIFF (test -n "$FORGE_MAX_COMMIT_DIFF"; and echo $FORGE_MAX_COMMIT_DIFF; or echo 100000)
set -g _FORGE_DELIMITER '\\s\\s+'
set -g _FORGE_PREVIEW_WINDOW "--preview-window=bottom:75%:wrap:border-sharp"
set -g _FORGE_CONVERSATION_ID ""
set -g _FORGE_PREVIOUS_CONVERSATION_ID ""
set -g _FORGE_ACTIVE_AGENT ""
set -g _FORGE_SESSION_MODEL ""
set -g _FORGE_SESSION_PROVIDER ""
set -g _FORGE_SESSION_REASONING_EFFORT ""
set -g _FORGE_COMMANDS ""

# Detect fd
if command -q fdfind
    set -g _FORGE_FD_CMD fdfind
else if command -q fd
    set -g _FORGE_FD_CMD fd
else
    set -g _FORGE_FD_CMD fd
end

# Detect bat
if command -q bat
    set -g _FORGE_CAT_CMD "bat --color=always --style=numbers,changes --line-range=:500"
else
    set -g _FORGE_CAT_CMD cat
end

# ── Key bindings ──────────────────────────────────────────────────────────────
# Override Enter to intercept :commands
bind \r _forge_accept_line
bind \n _forge_accept_line

# Tab completion for @file and :command
bind \t _forge_tab_completion

# ── Syntax highlighting for :commands ─────────────────────────────────────────
# Fish's built-in highlighter colors unknown commands red. Since :commands like
# :kb, :model, :suggest are intercepted by our Enter key handler (not real
# commands), they'd always appear red. We register a regex abbreviation that
# matches any `:word` token in command position. The highlighter checks
# abbreviations when deciding if a command is valid, so :commands get the
# normal command color. The abbreviation function returns the input unchanged
# so it never visually expands -- our Enter keybinding fires first anyway.
abbr --erase _forge_cmd 2>/dev/null
abbr --add _forge_cmd --position command --regex ":[a-zA-Z][-a-zA-Z0-9_]*" --function _forge_highlight_noop

# ── Right prompt integration ──────────────────────────────────────────────────
# Wrap the existing fish_right_prompt (e.g. starship) to append forge info.
# We defer this so other conf.d scripts (starship) can set up first.
if not set -q _FORGE_THEME_LOADED
    function _forge_install_rprompt --on-event fish_prompt
        # Only run once
        functions -e _forge_install_rprompt

        if functions -q fish_right_prompt
            # Save the original right prompt
            functions -c fish_right_prompt _forge_original_fish_right_prompt

            # Redefine with forge info appended
            function fish_right_prompt
                set -l forge_info (_forge_rprompt_info)
                set -l original (_forge_original_fish_right_prompt)
                if test -n "$forge_info"
                    echo -n "$forge_info "
                end
                echo -n "$original"
            end
        else
            function fish_right_prompt
                _forge_rprompt_info
            end
        end
    end
    set -g _FORGE_THEME_LOADED (date +%s)
end

set -g _FORGE_PLUGIN_LOADED (date +%s)
