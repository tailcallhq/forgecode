# Forge: _forge_tab_completion - Tab handler for @file and :command completion
function _forge_tab_completion
    set -l buf (commandline)
    set -l cursor_pos (commandline -C)

    # Get the word under/before cursor
    set -l tokens (commandline -co)
    set -l current_token (commandline -ct)

    # @file completion
    if string match -rq '^@' -- "$current_token"
        set -l filter_text (string replace -r '^@' '' -- "$current_token")
        set -l fzf_args \
            --preview="if test -d {}; ls -la {} 2>/dev/null; else; $_FORGE_CAT_CMD {}; end" \
            $_FORGE_PREVIEW_WINDOW

        set -l file_list ($_FORGE_FD_CMD --type f --type d --hidden --exclude .git)

        set -l selected
        if test -n "$filter_text"
            set selected (printf '%s\n' $file_list | _forge_fzf --query "$filter_text" $fzf_args)
        else
            set selected (printf '%s\n' $file_list | _forge_fzf $fzf_args)
        end

        if test -n "$selected"
            set selected "@[$selected]"
            # Replace the current @token with the selected file
            set -l before (string replace -r '@[^@]*$' '' -- (commandline -cb))
            commandline -r "$before$selected"(string replace -r '^[^ ]*' '' -- (commandline | string sub -s (math $cursor_pos + 1)))
            commandline -C (math (string length "$before$selected"))
        end
        commandline -f repaint
        return
    end

    # :command completion - only if the entire buffer is `:something`
    if string match -rq '^:([a-zA-Z][a-zA-Z0-9_-]*)?$' -- "$buf"
        set -l filter_text (string replace -r '^:' '' -- "$buf")
        set -l commands_list (_forge_get_commands)

        if test -n "$commands_list"
            # If user typed a partial command, try inline completion first
            if test -n "$filter_text"
                # Build list of all completable names (commands + aliases)
                # Each line: "name" where name is either the command or an alias
                set -l all_names
                for line in (printf '%s\n' $commands_list | tail -n +2)
                    set -l cmd_name (echo "$line" | awk '{print $1}')
                    set all_names $all_names $cmd_name
                    # Extract aliases from "[alias: x]" or "[alias: x, y]" or "[alias for: x]"
                    set -l aliases (echo "$line" | string match -r '\[alias(?:\s+for)?:\s*([^\]]+)\]' | tail -n 1)
                    if test -n "$aliases"
                        for a in (string split ',' -- $aliases)
                            set -l trimmed (string trim -- $a)
                            if test -n "$trimmed"
                                set all_names $all_names $trimmed
                            end
                        end
                    end
                end

                # Filter to matching names
                set -l matches
                for name in $all_names
                    if string match -riq "^$filter_text" -- $name
                        set matches $matches $name
                    end
                end
                # Deduplicate
                set matches (printf '%s\n' $matches | sort -u)
                set -l match_count (count $matches)

                if test $match_count -eq 1
                    # Exact single match -- complete inline
                    commandline -r ":$matches[1] "
                    commandline -C (math (string length ":$matches[1] "))
                    commandline -f repaint
                    return
                else if test $match_count -gt 1
                    # Find longest common prefix among matches
                    set -l prefix $matches[1]
                    for m in $matches[2..]
                        set -l new_prefix ""
                        set -l plen (string length "$prefix")
                        set -l mlen (string length "$m")
                        set -l len (math "min($plen, $mlen)")
                        for idx in (seq 1 $len)
                            set -l pc (string lower (string sub -s $idx -l 1 "$prefix"))
                            set -l mc (string lower (string sub -s $idx -l 1 "$m"))
                            if test "$pc" = "$mc"
                                set new_prefix "$new_prefix"(string sub -s $idx -l 1 "$prefix")
                            else
                                break
                            end
                        end
                        set prefix $new_prefix
                    end

                    # If prefix is longer than what user typed, extend it
                    if test (string length "$prefix") -gt (string length "$filter_text")
                        commandline -r ":$prefix"
                        commandline -C (math (string length ":$prefix"))
                        commandline -f repaint
                        return
                    end

                    # Otherwise fall through to fzf picker
                end
            end

            # Bare ":" or ambiguous partial -- open fzf picker
            set -l selected
            if test -n "$filter_text"
                set selected (printf '%s\n' $commands_list | _forge_fzf --header-lines=1 --delimiter="$_FORGE_DELIMITER" --nth=1 --query "$filter_text" --prompt="Command ❯ ")
            else
                set selected (printf '%s\n' $commands_list | _forge_fzf --header-lines=1 --delimiter="$_FORGE_DELIMITER" --nth=1 --prompt="Command ❯ ")
            end

            if test -n "$selected"
                set -l command_name (echo "$selected" | awk '{print $1}')
                commandline -r ":$command_name "
                commandline -C (math (string length ":$command_name "))
            end
        end
        commandline -f repaint
        return
    end

    # Default: normal tab completion
    commandline -f complete
end
