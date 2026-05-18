#!/usr/bin/env python3
"""End-to-end tests for MCP server permission policy (PR #3324)."""

import contextlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

try:
    import pexpect
except ImportError:
    subprocess.check_call([sys.executable, "-m", "pip", "install", "pexpect"])
    import pexpect

try:
    import yaml
except ImportError:
    subprocess.check_call([sys.executable, "-m", "pip", "install", "pyyaml"])
    import yaml

FORGE_BIN = os.path.abspath(os.environ.get("FORGE_BIN", "forge"))
PASS = "\033[32mPASS\033[0m"
FAIL = "\033[31mFAIL\033[0m"
results: list = []


@contextlib.contextmanager
def scenario_dirs():
    cwd = tempfile.mkdtemp(prefix="forge_mcp_cwd_")
    cfg = tempfile.mkdtemp(prefix="forge_mcp_cfg_")
    for base in [os.path.expanduser("~/forge"), os.path.expanduser("~/.forge")]:
        if os.path.isdir(base):
            for name in [".forge.toml", ".config.json", ".credentials.json"]:
                src = os.path.join(base, name)
                if os.path.exists(src):
                    shutil.copy2(src, os.path.join(cfg, name))
            break
    try:
        yield cwd, cfg
    finally:
        shutil.rmtree(cwd, ignore_errors=True)
        shutil.rmtree(cfg, ignore_errors=True)


def read_perms(cfg: str) -> dict:
    p = os.path.join(cfg, "permissions.yaml")
    return yaml.safe_load(open(p)) or {} if os.path.exists(p) else {}


def write_perms(cfg: str, data: dict):
    with open(os.path.join(cfg, "permissions.yaml"), "w") as f:
        yaml.dump(data, f, default_flow_style=False)


def write_mcp(path: str, command: str, args=None, key="test-server"):
    server: dict = {"command": command}
    if args:
        server["args"] = args
    with open(os.path.join(path, ".mcp.json"), "w") as f:
        json.dump({"mcpServers": {key: server}}, f)


def spawn(cwd: str, cfg: str) -> pexpect.spawn:
    env = {**os.environ, "TERM": "xterm-256color", "COLUMNS": "120", "LINES": "40", "FORGE_CONFIG": cfg}
    return pexpect.spawn(
        "/bin/sh", args=["-c", f"exec {FORGE_BIN} -p hello 2>&1"],
        cwd=cwd, timeout=30, encoding="utf-8", codec_errors="replace", env=env,
    )


def accept(child: pexpect.spawn):
    child.expect("Allow MCP server", timeout=30)
    child.send("\r")
    time.sleep(4)
    child.close(force=True)


def reject(child: pexpect.spawn):
    child.expect("Allow MCP server", timeout=30)
    for ch in ("\x1b", "[", "B"):  # arrow-down as separate bytes for raw-mode TUI
        child.send(ch)
        time.sleep(0.1)
    time.sleep(0.4)
    child.send("\r")
    time.sleep(4)
    child.close(force=True)


def no_prompt(child: pexpect.spawn) -> bool:
    idx = child.expect(["Allow MCP server", pexpect.TIMEOUT, pexpect.EOF], timeout=15)
    child.close(force=True)
    return idx != 0


def mcp_rules(perms: dict, permission: str) -> list:
    return [
        p for p in perms.get("policies", [])
        if isinstance(p, dict) and p.get("permission") == permission
        and isinstance(p.get("rule"), dict) and "mcp" in p["rule"]
    ]


def show_perms(before: dict, after: dict):
    def dump(d):
        if not d:
            print("    (empty)")
            return
        for line in yaml.dump(d, default_flow_style=False, sort_keys=False).splitlines():
            print(f"    {line}")
    print("  ┌─ before ────────────────────────")
    dump(before)
    print("  ├─ after  ────────────────────────")
    dump(after)
    print("  └─────────────────────────────────")


def run(name: str, fn):
    print(f"\n{'─'*50}\n{name}\n{'─'*50}")
    try:
        fn()
        print(f"  {PASS}")
        results.append((name, True, None))
    except Exception as e:
        print(f"  {FAIL} — {e}")
        results.append((name, False, str(e)))


# ---------------------------------------------------------------------------
# Scenarios
# ---------------------------------------------------------------------------

def test_accept_writes_allow():
    with scenario_dirs() as (cwd, cfg):
        write_mcp(cwd, "echo", ["hello"])
        before = read_perms(cfg)
        accept(spawn(cwd, cfg))
        after = read_perms(cfg)
        show_perms(before, after)
        assert mcp_rules(after, "allow"), "Expected an MCP allow rule"


def test_reject_writes_deny():
    with scenario_dirs() as (cwd, cfg):
        write_mcp(cwd, "echo", ["hello"])
        before = read_perms(cfg)
        reject(spawn(cwd, cfg))
        after = read_perms(cfg)
        show_perms(before, after)
        assert mcp_rules(after, "deny"), "Expected an MCP deny rule"


def test_existing_allow_skips_prompt():
    with scenario_dirs() as (cwd, cfg):
        write_perms(cfg, {"policies": [{"permission": "allow", "rule": {"mcp": {"command": "echo"}}}]})
        write_mcp(cwd, "echo", ["hello"])
        before = read_perms(cfg)
        assert no_prompt(spawn(cwd, cfg)), "Prompt appeared even though allow rule was pre-written"
        show_perms(before, read_perms(cfg))


def test_second_run_skips_prompt():
    with scenario_dirs() as (cwd, cfg):
        write_mcp(cwd, "echo", ["hello"])

        before1 = read_perms(cfg)
        accept(spawn(cwd, cfg))
        print("  [run 1]"); show_perms(before1, read_perms(cfg))

        before2 = read_perms(cfg)
        assert no_prompt(spawn(cwd, cfg)), "Prompt appeared on second run — decision was not persisted"
        print("  [run 2]"); show_perms(before2, read_perms(cfg))


def test_npx_accept():
    with scenario_dirs() as (cwd, cfg):
        write_mcp(cwd, "npx", ["-y", "@modelcontextprotocol/server-filesystem", cwd], key="filesystem")
        before = read_perms(cfg)
        accept(spawn(cwd, cfg))
        after = read_perms(cfg)
        show_perms(before, after)
        rules = mcp_rules(after, "allow")
        assert rules and rules[0]["rule"]["mcp"].get("command") == "npx", "Expected npx allow rule"


def test_npx_reject():
    with scenario_dirs() as (cwd, cfg):
        write_mcp(cwd, "npx", ["-y", "@modelcontextprotocol/server-filesystem", cwd], key="filesystem")
        before = read_perms(cfg)
        reject(spawn(cwd, cfg))
        after = read_perms(cfg)
        show_perms(before, after)
        rules = mcp_rules(after, "deny")
        assert rules and rules[0]["rule"]["mcp"].get("command") == "npx", "Expected npx deny rule"


def test_user_scope_never_prompts():
    with scenario_dirs() as (cwd, cfg):
        write_mcp(cfg, "echo", ["user-scope"], key="user-server")  # inside cfg = user scope
        before = read_perms(cfg)
        assert no_prompt(spawn(cwd, cfg)), "Prompt appeared for user-scope server"
        show_perms(before, read_perms(cfg))


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    print(f"{'='*50}\nMCP Permission Policy — E2E Tests\n{'='*50}")
    print(f"  binary: {FORGE_BIN}\n")

    run("Accept → allow rule written",          test_accept_writes_allow)
    run("Reject → deny rule written",           test_reject_writes_deny)
    run("Pre-existing allow → no prompt",       test_existing_allow_skips_prompt)
    run("Second run after accept → no prompt",  test_second_run_skips_prompt)
    run("npx server Accept → allow rule",       test_npx_accept)
    run("npx server Reject → deny rule",        test_npx_reject)
    run("User-scope server → never prompts",    test_user_scope_never_prompts)

    passed = sum(1 for _, ok, _ in results if ok)
    print(f"\n{'='*50}\nSUMMARY\n{'='*50}")
    for name, ok, err in results:
        print(f"  {PASS if ok else FAIL}  {name}")
        if err:
            print(f"       {err}")
    print(f"\n{passed}/{len(results)} passed")
    sys.exit(0 if passed == len(results) else 1)


if __name__ == "__main__":
    main()
