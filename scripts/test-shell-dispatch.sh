#!/usr/bin/env zsh

# Regression tests for the executable selected by the Forgecode zsh integration.

set -euo pipefail

SCRIPT_DIR="${0:A:h}"
REPO_ROOT="${SCRIPT_DIR:h}"

PASS=0
FAIL=0

function assert_eq() {
    local test_name="$1"
    local actual="$2"
    local expected="$3"

    if [[ "$actual" == "$expected" ]]; then
        print -r -- "[PASS] ${test_name}"
        PASS=$((PASS + 1))
    else
        print -r -- "[FAIL] ${test_name}"
        print -r -- "  expected: ${expected}"
        print -r -- "    actual: ${actual}"
        FAIL=$((FAIL + 1))
    fi
}

function assert_contains() {
    local test_name="$1"
    local actual="$2"
    local expected="$3"

    if [[ "$actual" == *"${expected}"* ]]; then
        print -r -- "[PASS] ${test_name}"
        PASS=$((PASS + 1))
    else
        print -r -- "[FAIL] ${test_name}"
        print -r -- "  expected to contain: ${expected}"
        FAIL=$((FAIL + 1))
    fi
}

function setup_output() {
    local forge_bin="${1:-}"

    FORGE_BIN="$forge_bin" zsh -dfc '
        forge() {
            print -u2 -r -- "forge:$*"
            case "$1:$2" in
                zsh:plugin) print -r -- "typeset -g _FORGE_PLUGIN_LOADED=1" ;;
                zsh:theme) print -r -- "typeset -g _FORGE_THEME_LOADED=1" ;;
            esac
        }
        helioslite() {
            print -u2 -r -- "helioslite:$*"
            case "$1:$2" in
                zsh:plugin) print -r -- "typeset -g _FORGE_PLUGIN_LOADED=1" ;;
                zsh:theme) print -r -- "typeset -g _FORGE_THEME_LOADED=1" ;;
            esac
        }
        source "$1"
        print -r -- "loaded:${_FORGE_PLUGIN_LOADED:-0}:${_FORGE_THEME_LOADED:-0}"
    ' _ "${REPO_ROOT}/shell-plugin/forge.setup.zsh" 2>&1
}

function doctor_output() {
    local forge_bin="${1:-}"

    FORGE_BIN="$forge_bin" ZDOTDIR="${REPO_ROOT}/.test-zdotdir" zsh -dfc '
        forge() {
            if [[ "$1" == "--version" ]]; then
                print -r -- "forge 1.0.0"
            fi
        }
        helioslite() {
            if [[ "$1" == "--version" ]]; then
                print -r -- "helioslite 9.9.9"
            fi
        }
        source "$1"
    ' _ "${REPO_ROOT}/shell-plugin/doctor.zsh" 2>&1 || true
}

function theme_output() {
    local forge_bin="${1:-}"

    FORGE_BIN="$forge_bin" zsh -dfc '
        forge() { print -r -- "forge:$*"; }
        helioslite() { print -r -- "helioslite:$*"; }
        source "$1"
        _forge_prompt_info
    ' _ "${REPO_ROOT}/shell-plugin/forge.theme.zsh"
}

assert_eq "plugin config defaults to helioslite" \
    "$(zsh -dfc 'source "$1"; print -r -- "$_FORGE_BIN"' _ "${REPO_ROOT}/shell-plugin/lib/config.zsh")" \
    "helioslite"

assert_eq "plugin config preserves FORGE_BIN override" \
    "$(FORGE_BIN="/opt/custom-forge" zsh -dfc 'source "$1"; print -r -- "$_FORGE_BIN"' _ "${REPO_ROOT}/shell-plugin/lib/config.zsh")" \
    "/opt/custom-forge"

assert_eq "setup loads plugin and theme through helioslite by default" \
    "$(setup_output)" \
    $'helioslite:zsh plugin\nhelioslite:zsh theme\nloaded:1:1'

assert_eq "setup preserves FORGE_BIN override" \
    "$(setup_output "forge")" \
    $'forge:zsh plugin\nforge:zsh theme\nloaded:1:1'

assert_eq "standalone theme defaults to helioslite" \
    "$(theme_output)" \
    "helioslite:zsh rprompt"

assert_eq "standalone theme preserves FORGE_BIN override" \
    "$(theme_output "forge")" \
    "forge:zsh rprompt"

default_doctor_output="$(doctor_output)"
assert_contains "doctor checks helioslite by default" "$default_doctor_output" "helioslite: 9.9.9"
assert_contains "doctor suggests helioslite plugin setup by default" "$default_doctor_output" '"helioslite" zsh plugin'

override_doctor_output="$(doctor_output "forge")"
assert_contains "doctor preserves FORGE_BIN override" "$override_doctor_output" "forge: 1.0.0"
assert_contains "doctor preserves FORGE_BIN override in hints" "$override_doctor_output" '"forge" zsh plugin'

print -r -- "${PASS}/$((PASS + FAIL)) checks passed"
((FAIL == 0))
