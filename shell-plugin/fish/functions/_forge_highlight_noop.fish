# Forge: _forge_highlight_noop - No-op abbreviation function for syntax highlighting
# Returns the input token unchanged. Used by the _forge_cmd regex abbreviation
# so Fish's highlighter recognizes :commands as valid (showing them in white
# instead of red). The actual :command dispatch is handled by _forge_accept_line.
function _forge_highlight_noop
    echo -- $argv[1]
end
