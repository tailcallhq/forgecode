#!/usr/bin/env zsh

# Enable prompt substitution for RPROMPT
setopt PROMPT_SUBST

# Model and agent info with token count
# Fully formatted output directly from Rust
# Returns ZSH-formatted string ready for use in RPROMPT
function _forge_prompt_info() {
    local forge_bin="${_FORGE_BIN:-${FORGE_BIN:-forge}}"
    
    # Get fully formatted prompt from forge (single command).
    # Pass session model/provider as CLI flags when set so the rprompt
    # reflects the active session override rather than global config.
    local -a forge_cmd
    forge_cmd=("$forge_bin")
    forge_cmd+=(zsh rprompt)
    [[ -n "$_FORGE_SESSION_MODEL" ]] && local -x FORGE_SESSION__MODEL_ID="$_FORGE_SESSION_MODEL"
    [[ -n "$_FORGE_SESSION_PROVIDER" ]] && local -x FORGE_SESSION__PROVIDER_ID="$_FORGE_SESSION_PROVIDER"
    [[ -n "$_FORGE_SESSION_REASONING_EFFORT" ]] && local -x FORGE_REASONING__EFFORT="$_FORGE_SESSION_REASONING_EFFORT"
    _FORGE_CONVERSATION_ID=$_FORGE_CONVERSATION_ID _FORGE_ACTIVE_AGENT=$_FORGE_ACTIVE_AGENT "${forge_cmd[@]}" 2>/dev/null
}

# Right prompt: agent and model with token count (uses single forge prompt command)
# Set RPROMPT if empty, otherwise append to existing value
if [[ -z "$_FORGE_THEME_LOADED" ]]; then
    RPROMPT='$(_forge_prompt_info)'"${RPROMPT:+ ${RPROMPT}}"
fi
