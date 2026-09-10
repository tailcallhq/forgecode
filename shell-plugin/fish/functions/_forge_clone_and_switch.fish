# Forge: _forge_clone_and_switch - Clone a conversation and switch to it
function _forge_clone_and_switch
    set -l clone_target $argv[1]
    set -l original_conversation_id $_FORGE_CONVERSATION_ID

    _forge_log info (set_color --bold)"Cloning conversation $clone_target"(set_color normal)

    set -l clone_output ($_FORGE_BIN conversation clone "$clone_target" 2>&1)
    set -l clone_exit $status

    if test $clone_exit -eq 0
        set -l new_id (printf '%s\n' $clone_output | grep -oE '[a-f0-9-]{36}' | tail -1)
        if test -n "$new_id"
            _forge_switch_conversation $new_id
            _forge_log success "└─ Switched to conversation "(set_color --bold)"$new_id"(set_color normal)
            if test "$clone_target" != "$original_conversation_id"
                echo
                _forge_exec conversation show $new_id
                echo
                _forge_exec conversation info $new_id
            end
        else
            _forge_log error "Failed to extract new conversation ID from clone output"
        end
    else
        _forge_log error "Failed to clone conversation: $clone_output"
    end
end
