# Forge: _forge_fzf - Wrapper around fzf with forge defaults
function _forge_fzf
    fzf --reverse --exact --cycle --select-1 --height 80% --no-scrollbar --ansi --color="header:bold" $argv
end
