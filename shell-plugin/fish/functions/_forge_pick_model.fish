# Forge: _forge_pick_model - Interactive model picker via fzf
# Usage: _forge_pick_model prompt_text current_model input_text [current_provider] [provider_field]
function _forge_pick_model
    set -l prompt_text $argv[1]
    set -l current_model $argv[2]
    set -l input_text $argv[3]
    set -l current_provider $argv[4]
    set -l provider_field $argv[5]

    set -l output ($_FORGE_BIN list models --porcelain 2>/dev/null)
    if test -z "$output"
        return 1
    end

    set -l fzf_args --delimiter="$_FORGE_DELIMITER" --prompt="$prompt_text" --with-nth="2,3,5.."
    if test -n "$input_text"
        set fzf_args $fzf_args --query="$input_text"
    end
    if test -n "$current_model"
        if test -n "$current_provider" -a -n "$provider_field"
            set -l idx (printf '%s\n' $output | _forge_find_index "$current_model" 1 "$provider_field" "$current_provider")
            set fzf_args $fzf_args --bind="start:pos($idx)"
        else
            set -l idx (printf '%s\n' $output | _forge_find_index "$current_model" 1)
            set fzf_args $fzf_args --bind="start:pos($idx)"
        end
    end

    printf '%s\n' $output | _forge_fzf --header-lines=1 $fzf_args
end
