# Forge: _forge_find_index - Find line index in porcelain output matching a value
# Usage: printf '%s\n' $data | _forge_find_index "value" [field_num] [field_num2] [value2]
# Reads multi-line data from stdin to avoid Fish list-to-string quoting issues.
function _forge_find_index
    set -l value_to_find $argv[1]
    set -l field_number (test -n "$argv[2]"; and echo $argv[2]; or echo 1)
    set -l field_number2 $argv[3]
    set -l value_to_find2 $argv[4]

    set -l index 1
    set -l line_num 0
    while read -l line
        set line_num (math $line_num + 1)
        if test $line_num -eq 1
            continue
        end
        set -l field_value (echo "$line" | awk -F '  +' "{print \$$field_number}")
        if test "$field_value" = "$value_to_find"
            if test -n "$field_number2" -a -n "$value_to_find2"
                set -l field_value2 (echo "$line" | awk -F '  +' "{print \$$field_number2}")
                if test "$field_value2" = "$value_to_find2"
                    echo $index
                    return 0
                end
            else
                echo $index
                return 0
            end
        end
        set index (math $index + 1)
    end
    echo 1
    return 0
end
