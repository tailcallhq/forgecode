# Forge: _forge_select_provider - Interactive provider selection via fzf
# Usage: _forge_select_provider [filter_status] [current_provider] [filter_type] [query]
function _forge_select_provider
    set -l filter_status $argv[1]
    set -l current_provider $argv[2]
    set -l filter_type $argv[3]
    set -l query $argv[4]

    set -l cmd $_FORGE_BIN list provider --porcelain
    if test -n "$filter_type"
        set cmd $cmd --type=$filter_type
    end

    set -l output ($cmd 2>/dev/null)
    if test -z "$output"
        _forge_log error "No providers available"
        return 1
    end

    if test -n "$filter_status"
        set -l header (printf '%s\n' $output | head -n 1)
        set -l filtered (printf '%s\n' $output | tail -n +2 | grep -i "$filter_status")
        if test -z "$filtered"
            _forge_log error "No $filter_status providers found"
            return 1
        end
        set output (printf "%s\n%s" "$header" "$filtered")
    end

    if test -z "$current_provider"
        set current_provider ($_FORGE_BIN config get provider --porcelain 2>/dev/null)
    end

    set -l fzf_args --delimiter="$_FORGE_DELIMITER" --prompt="Provider ❯ " --with-nth=1,3..
    if test -n "$query"
        set fzf_args $fzf_args --query="$query"
    end
    if test -n "$current_provider"
        set -l idx (printf '%s\n' $output | _forge_find_index "$current_provider" 1)
        set fzf_args $fzf_args --bind="start:pos($idx)"
    end

    set -l selected (printf '%s\n' $output | _forge_fzf --header-lines=1 $fzf_args)
    if test -n "$selected"
        echo "$selected"
        return 0
    end
    return 1
end
