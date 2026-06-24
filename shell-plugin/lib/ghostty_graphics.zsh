#!/usr/bin/env zsh

#:# ZSH glue for Ghostty graphics — manifest loader, show/hide/cycle, pre-compile
#:#
#:# Load with: `autoload -U forge_graphics_detect && forge_graphics_detect`
#:#
#:# Functions provided:
#:#   forge_graphics_detect      — probe for Ghostty graphics capability
#:#   forge_graphics_load_manifest — load `.gph`/JSON manifest into env
#:#   forge_graphics_show        — show named graphic (cached 1s)
#:#   forge_graphics_hide        — hide graphic or all
#:#   forge_graphics_cycle       — rotate graphics in background
#:#   forge_graphics_compile     — pre-compile GLSL → .gph

# Guard against double-sourcing
[[ -n "${_FORGE_GRAPHICS_ZSH_LOADED:-}" ]] && return 0
typeset -g _FORGE_GRAPHICS_ZSH_LOADED=1

emulate -L zsh
setopt local_options pipe_fail no_unset

# ---------------------------------------------------------------------------
# Private config
# ---------------------------------------------------------------------------

# Binary the wrappers call. Overridable for testing (FORGE_GHOSTTY_BIN=...).
typeset -g _FORGE_GHOSTTY_BIN="${FORGE_GHOSTTY_BIN:-ghostty}"
# 1-second dedupe window for show/hide to avoid terminal flicker.
typeset -g _FORGE_GRAPHICS_CACHE_TTL=1
typeset -g _FORGE_GRAPHICS_LAST_SHOW_TS=0 _FORGE_GRAPHICS_LAST_SHOW_NAME=""
typeset -g _FORGE_GRAPHICS_LAST_HIDE_TS=0 _FORGE_GRAPHICS_LAST_HIDE_NAME=""
# PID of the most recent `forge_graphics_cycle` background subshell.
typeset -g _FORGE_GRAPHICS_CYCLE_PID=""
typeset -g _FORGE_GRAPHICS_LAST_COMPILE_HASH=""

# ---------------------------------------------------------------------------
# Private helpers
# ---------------------------------------------------------------------------

# Append a single line to $_FORGE_GRAPHICS_LOG when the user has set it.
function _forge_graphics_log() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset
    [[ -z "${_FORGE_GRAPHICS_LOG:-}" ]] && return 0
    print -- "[$(date +%s)] forge_graphics: $*" >> "$_FORGE_GRAPHICS_LOG" 2>/dev/null
    return 0
}

# Run the ghostty binary, redirecting output to /dev/tty when called from a
# ZLE widget so we don't corrupt the line buffer. Returns the binary's exit code.
function _forge_graphics_invoke() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset
    if [[ -n "${WIDGET:-}" ]]; then
        "$_FORGE_GHOSTTY_BIN" "$@" >/dev/tty 2>/dev/tty
    else
        "$_FORGE_GHOSTTY_BIN" "$@" 2>/dev/tty
    fi
}

# sha256 of a file; empty string on failure. Uses python3 to avoid external deps.
function _forge_graphics_sha256() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset
    python3 -c 'import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$1" 2>/dev/null
}

# ---------------------------------------------------------------------------
# Search dirs
# ---------------------------------------------------------------------------

# Search the well-known graphics dirs and echo the first one that contains
# `manifest.json` or at least one `*.gph` file. Returns 0 and prints the
# path on success, 1 (prints nothing) on miss.
# Priority: ${XDG_DATA_HOME:-$HOME/.local/share}/forge/graphics,
#           ${FORGE_ROOT}/graphics/,  ./graphics/
function _forge_graphics_search_dirs() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local -a candidates=(
        "${XDG_DATA_HOME:-$HOME/.local/share}/forge/graphics"
        "${FORGE_ROOT:-}/graphics"
        "./graphics"
    )
    local dir gphs
    for dir in "${candidates[@]}"; do
        [[ -d "$dir" ]] || continue
        if [[ -f "$dir/manifest.json" ]]; then
            print -- "$dir"
            return 0
        fi
        gphs=("$dir"/*.gph(.N))
        if (( ${#gphs} > 0 )); then
            print -- "$dir"
            return 0
        fi
    done
    return 1
}

# ---------------------------------------------------------------------------
# Detection
# ---------------------------------------------------------------------------

# Detect Ghostty graphics support. Sets FORGE_GRAPHICS_AVAILABLE=1 on success.
# Strategy 1: `ghostty` on PATH and its --help advertises +graphics-show.
# Strategy 2: ghostty.zsh already declared FORGE_GHOSTTY_AVAILABLE=1.
function forge_graphics_detect() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local detected=0
    if (( $+commands[ghostty] )); then
        local help_out
        help_out=$("$_FORGE_GHOSTTY_BIN" --help 2>&1) || help_out=""
        [[ "$help_out" == *graphics-show* ]] && detected=1
    fi
    if (( detected == 0 )) && [[ "${FORGE_GHOSTTY_AVAILABLE:-0}" == "1" ]]; then
        detected=1
    fi

    if (( detected == 1 )); then
        typeset -g FORGE_GRAPHICS_AVAILABLE=1
        _forge_graphics_log "detect: available"
        return 0
    fi
    typeset -g FORGE_GRAPHICS_AVAILABLE=0
    _forge_graphics_log "detect: not available"
    return 1
}

# ---------------------------------------------------------------------------
# Manifest loader
# ---------------------------------------------------------------------------

# Load a graphics manifest and source entries into the current shell.
# Usage: forge_graphics_load_manifest [<path>]
#   <path>: a manifest.json, a .gph file, a directory of .gph files, or empty
#           (consults _forge_graphics_search_dirs).
# Sets: FORGE_GRAPHICS_COUNT, FORGE_GRAPHICS_<NAME>_PATH/_HASH,
#       _FORGE_GRAPHICS_<NAME>_HASH (drift-check baseline).
# Returns 0 on success, 1 on missing path / invalid JSON / unreadable file.
function forge_graphics_load_manifest() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local target="${1:-}"
    if [[ -z "$target" ]]; then
        target=$(_forge_graphics_search_dirs) || {
            _forge_graphics_log "load_manifest: no path and no search-dir hit"
            return 1
        }
    fi

    local manifest_file=""
    if [[ -d "$target" ]]; then
        if [[ -f "$target/manifest.json" ]]; then
            manifest_file="$target/manifest.json"
        else
            # Directory-of-.gph: synthesize a single-entry manifest.
            local -a gphs
            gphs=("$target"/*.gph(.N))
            if (( ${#gphs} == 0 )); then
                _forge_graphics_log "load_manifest: directory has no .gph files: $target"
                return 1
            fi
            local first="${gphs[1]}"
            local name="${${first:t}%.gph}"
            local upper="${(U)name}"
            local hash
            hash=$(_forge_graphics_sha256 "$first")
            typeset -g FORGE_GRAPHICS_COUNT=1
            typeset -g "FORGE_GRAPHICS_${upper}_PATH=$first"
            typeset -g "FORGE_GRAPHICS_${upper}_HASH=$hash"
            typeset -g "_FORGE_GRAPHICS_${upper}_HASH=$hash"
            _forge_graphics_log "load_manifest: dir=$target single-gph=$name hash=${hash:0:8}"
            return 0
        fi
    elif [[ -f "$target" ]]; then
        manifest_file="$target"
    else
        _forge_graphics_log "load_manifest: path does not exist: $target"
        return 1
    fi

    # Validate JSON. Prefer `jq` if present; fall back to a python3 one-liner.
    if (( $+commands[jq] )); then
        if ! jq empty "$manifest_file" 2>/dev/null; then
            _forge_graphics_log "load_manifest: invalid JSON (jq): $manifest_file"
            return 1
        fi
    else
        if ! python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$manifest_file" 2>/dev/null; then
            _forge_graphics_log "load_manifest: invalid JSON (python3): $manifest_file"
            return 1
        fi
    fi

    # Parse + hash + emit shell assignments. Accepted manifest shapes:
    #   { "graphics": [{"name":..., "path":..., "hash":...}, ...] }
    #   [ {"name":..., "path":..., "hash":...}, ... ]
    local assignments
    assignments=$(FORGE_MANIFEST="$manifest_file" python3 <<'PYEOF' 2>/dev/null
import json, os, hashlib, re, sys
try:
    with open(os.environ["FORGE_MANIFEST"]) as f:
        data = json.load(f)
except Exception:
    print("ERR", file=sys.stderr); sys.exit(1)
base = os.path.dirname(os.path.abspath(os.environ["FORGE_MANIFEST"]))
graphics = data if isinstance(data, list) else data.get("graphics", [])
print(f'typeset -g FORGE_GRAPHICS_COUNT={len(graphics)}')
for e in graphics:
    name, path, declared = e.get("name",""), e.get("path",""), e.get("hash","")
    if not name or not path: continue
    if not os.path.isabs(path): path = os.path.normpath(os.path.join(base, path))
    actual = hashlib.sha256(open(path,"rb").read()).hexdigest() if os.path.isfile(path) else ""
    if declared and actual and declared != actual:
        print(f"# WARN hash drift for {name}: declared={declared[:8]} actual={actual[:8]}", file=sys.stderr)
    safe = re.sub(r'[^A-Za-z0-9_]', '_', name).upper()
    print(f'typeset -g "FORGE_GRAPHICS_{safe}_PATH={path}"')
    print(f'typeset -g "FORGE_GRAPHICS_{safe}_HASH={actual}"')
    print(f'typeset -g "_FORGE_GRAPHICS_{safe}_HASH={actual}"')
PYEOF
    )
    if [[ $? -ne 0 || -z "$assignments" || "$assignments" == "ERR" ]]; then
        _forge_graphics_log "load_manifest: parser failed for $manifest_file"
        return 1
    fi

    eval "$assignments"
    _forge_graphics_log "load_manifest: ok file=$manifest_file count=${FORGE_GRAPHICS_COUNT:-0}"
    return 0
}

# ---------------------------------------------------------------------------
# Show / hide / cycle
# ---------------------------------------------------------------------------

# Run `ghostty +graphics-show <path>` for the named graphic. Returns 1 if
# FORGE_GRAPHICS_AVAILABLE != 1, if no manifest entry exists, or if the file's
# current sha256 has drifted from the captured _FORGE_GRAPHICS_<NAME>_HASH.
# Caches successful invocations for 1s to dedupe repeat calls.
function forge_graphics_show() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    if [[ "${FORGE_GRAPHICS_AVAILABLE:-0}" != "1" ]]; then
        _forge_graphics_log "show: graphics not available"
        return 1
    fi
    local name="${1:-}"
    [[ -z "$name" ]] && { print -u2 -- "forge_graphics_show: name argument required"; return 1; }

    local upper="${(U)name}" path_var="FORGE_GRAPHICS_${upper}_PATH"
    local path="${(P)path_var:-}"
    if [[ -z "$path" ]]; then
        _forge_graphics_log "show: no manifest entry for '$name'"
        print -u2 -- "forge_graphics_show: no graphics entry named '$name' (load manifest first?)"
        return 1
    fi

    # Drift check: refuse to show if file content changed since manifest load.
    local internal_hash="${(P)_FORGE_GRAPHICS_${upper}_HASH:-}"
    if [[ -n "$internal_hash" && -f "$path" ]]; then
        local current_hash
        current_hash=$(_forge_graphics_sha256 "$path")
        if [[ -n "$current_hash" && "$current_hash" != "$internal_hash" ]]; then
            _forge_graphics_log "show: hash drift for $name (loaded=${internal_hash:0:8} now=${current_hash:0:8})"
            print -u2 -- "forge_graphics_show: file content drifted for '$name'; reload manifest"
            return 1
        fi
    fi

    # 1-second dedupe cache.
    local now
    now=$(date +%s)
    if (( now - _FORGE_GRAPHICS_LAST_SHOW_TS < _FORGE_GRAPHICS_CACHE_TTL )) \
        && [[ "$_FORGE_GRAPHICS_LAST_SHOW_NAME" == "$name" ]]; then
        _forge_graphics_log "show: cache hit name=$name"
        return 0
    fi

    local rc
    _forge_graphics_invoke +graphics-show "$path"
    rc=$?

    if (( rc == 0 )); then
        _FORGE_GRAPHICS_LAST_SHOW_TS=$now
        _FORGE_GRAPHICS_LAST_SHOW_NAME="$name"
        _forge_graphics_log "show: ok name=$name path=$path"
    else
        _forge_graphics_log "show: ghostty returned $rc name=$name"
    fi
    return $rc
}

# Run `ghostty +graphics-hide [<name>]`. With no name, hides all.
# Same cache semantics as forge_graphics_show.
function forge_graphics_hide() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    if [[ "${FORGE_GRAPHICS_AVAILABLE:-0}" != "1" ]]; then
        _forge_graphics_log "hide: graphics not available"
        return 1
    fi
    local name="${1:-}" now rc
    now=$(date +%s)
    if (( now - _FORGE_GRAPHICS_LAST_HIDE_TS < _FORGE_GRAPHICS_CACHE_TTL )) \
        && [[ "$_FORGE_GRAPHICS_LAST_HIDE_NAME" == "$name" ]]; then
        _forge_graphics_log "hide: cache hit name=$name"
        return 0
    fi

    if [[ -n "$name" ]]; then
        _forge_graphics_invoke +graphics-hide "$name"
    else
        _forge_graphics_invoke +graphics-hide
    fi
    rc=$?

    if (( rc == 0 )); then
        _FORGE_GRAPHICS_LAST_HIDE_TS=$now
        _FORGE_GRAPHICS_LAST_HIDE_NAME="$name"
        _forge_graphics_log "hide: ok name=$name"
    else
        _forge_graphics_log "hide: ghostty returned $rc name=$name"
    fi
    return $rc
}

# Background subshell that rotates through every loaded graphic, showing each
# for <interval_s> seconds (default 30). The PID is captured in
# _FORGE_GRAPHICS_CYCLE_PID so the caller can stop the rotation.
function forge_graphics_cycle() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    if [[ "${FORGE_GRAPHICS_AVAILABLE:-0}" != "1" ]]; then
        _forge_graphics_log "cycle: graphics not available"
        return 1
    fi
    local interval="${1:-30}"
    if ! [[ "$interval" == <-> ]] || (( interval < 1 )); then
        print -u2 -- "forge_graphics_cycle: interval must be a positive integer (got '$interval')"
        return 1
    fi

    # Snapshot every FORGE_GRAPHICS_*_PATH into a shell snippet, eval it in
    # the subshell so each rotation child sees the same env without fork-bomb.
    local snapshot="" p val
    for p in ${(k)parameters}; do
        if [[ "$p" == FORGE_GRAPHICS_*_PATH ]]; then
            val="${(P)p}"
            snapshot+="${(qq)p}=${(qq)val}"$'\n'
        fi
    done
    if [[ -z "$snapshot" ]]; then
        _forge_graphics_log "cycle: no graphics loaded (manifest empty)"
        print -u2 -- "forge_graphics_cycle: no graphics loaded (call forge_graphics_load_manifest first)"
        return 1
    fi

    local widget_at_fork="${WIDGET:-}"

    (
        emulate -L zsh
        setopt local_options pipe_fail no_unset
        eval "$snapshot"
        local -a paths=()
        local v
        for v in ${(k)parameters}; do
            [[ "$v" == FORGE_GRAPHICS_*_PATH ]] || continue
            paths+=("${(P)v}")
        done
        (( ${#paths} == 0 )) && exit 0
        [[ -n "$widget_at_fork" ]] && WIDGET="$widget_at_fork"
        while true; do
            local path
            for path in "${paths[@]}"; do
                [[ -f "$path" ]] || continue
                _forge_graphics_invoke +graphics-show "$path"
                sleep "$interval"
            done
        done
    ) &

    typeset -g _FORGE_GRAPHICS_CYCLE_PID=$!
    _forge_graphics_log "cycle: started pid=$_FORGE_GRAPHICS_CYCLE_PID interval=${interval}s"
    return 0
}

# ---------------------------------------------------------------------------
# Pre-compile wrapper
# ---------------------------------------------------------------------------

# Pre-compile a GLSL shader into the Ghostty .gph graphics manifest.
# Usage: forge_graphics_compile <shader.glsl> <output.gph>
# Requires glslangValidator AND a ghostty binary that advertises
# +graphics-compile (forward-looking — gracefully returns 1 when absent).
# On success, _FORGE_GRAPHICS_LAST_COMPILE_HASH is the sha256 of the output.
function forge_graphics_compile() {
    emulate -L zsh
    setopt local_options pipe_fail no_unset

    local shader="${1:-}" output="${2:-}"
    if [[ -z "$shader" || -z "$output" ]]; then
        print -u2 -- "forge_graphics_compile: usage: forge_graphics_compile <shader.glsl> <output.gph>"
        return 1
    fi
    [[ -r "$shader" ]] || { print -u2 -- "forge_graphics_compile: shader file not readable: $shader"; return 1; }

    local help_out
    help_out=$("$_FORGE_GHOSTTY_BIN" --help 2>&1) || help_out=""
    if [[ "$help_out" != *graphics-compile* ]]; then
        _forge_graphics_log "compile: ghostty does not advertise +graphics-compile (forward-looking)"
        print -u2 -- "forge_graphics_compile: this ghostty build does not advertise +graphics-compile"
        return 1
    fi
    (( $+commands[glslangValidator] )) || {
        _forge_graphics_log "compile: glslangValidator not on PATH"
        print -u2 -- "forge_graphics_compile: glslangValidator not found on PATH"
        return 1
    }

    # GLSL → SPIR-V → .gph.
    local spirv="${output}.spv" rc
    glslangValidator -V "$shader" -o "$spirv" 2>/dev/null || {
        _forge_graphics_log "compile: glslangValidator failed for $shader"
        return 1
    }
    _forge_graphics_invoke +graphics-compile "$spirv" "$output"
    rc=$?
    rm -f "$spirv"

    if (( rc == 0 )); then
        local hash
        hash=$(_forge_graphics_sha256 "$output")
        typeset -g _FORGE_GRAPHICS_LAST_COMPILE_HASH="$hash"
        _forge_graphics_log "compile: ok shader=$shader output=$output hash=${hash:0:8}"
    else
        _forge_graphics_log "compile: ghostty +graphics-compile returned $rc"
    fi
    return $rc
}