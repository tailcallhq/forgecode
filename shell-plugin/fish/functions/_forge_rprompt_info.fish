# Forge: _forge_rprompt_info - Right prompt info showing model and agent
# Called by the fish_right_prompt wrapper installed in conf.d/forge.fish
# Builds the prompt natively in Fish (forge zsh rprompt outputs ZSH escapes)

function _forge_rprompt_info
    set -l forge_bin (test -n "$_FORGE_BIN"; and echo $_FORGE_BIN; or echo forge)
    set -l parts

    # Agent info
    set -l agent_name
    if test -n "$_FORGE_ACTIVE_AGENT"
        set agent_name (string upper $_FORGE_ACTIVE_AGENT)
    else
        set agent_name FORGE
    end
    set -a parts (set_color --bold 888888)"$agent_name"(set_color normal)

    # Model info
    set -l model_id
    if test -n "$_FORGE_SESSION_MODEL"
        set model_id $_FORGE_SESSION_MODEL
    else
        set model_id ($forge_bin config get model --porcelain 2>/dev/null)
    end
    if test -n "$model_id"
        set -a parts (set_color 888888)" $model_id"(set_color normal)
    end

    # Conversation indicator
    if test -n "$_FORGE_CONVERSATION_ID"
        set -a parts (set_color 888888)" "(set_color normal)
    end

    # Reasoning effort (only if session override)
    if test -n "$_FORGE_SESSION_REASONING_EFFORT"
        set -a parts (set_color yellow)" $_FORGE_SESSION_REASONING_EFFORT"(set_color normal)
    end

    echo -n (string join '' $parts)
end
