#!/usr/bin/env python3
"""
End-to-end tests for MCP server permission policy (PR #3324).

The forge permission TUI renders on stderr using crossterm raw mode.
pexpect.spawn() allocates a PTY so forge sees a real terminal, which lets
the TUI actually start. We send:
  - Enter         → select the first item (Accept by default)
  - Down + Enter  → select the second item (Reject)

After each interaction we assert on the contents of permissions.yaml.
"""

import os
import sys
import json
import shutil
import tempfile
import subprocess
import textwrap
import time

# ---------------------------------------------------------------------------
# Dependency bootstrap
# ---------------------------------------------------------------------------
try:
    import pexpect
except ImportError:
    subprocess.check_call([sys.executable, "-m", "pip", "install", "pexpect"])
    import pexpect

import yaml  # noqa: E402  (installed below if missing)
try:
    import yaml
except ImportError:
    subprocess.check_call([sys.executable, "-m", "pip", "install", "pyyaml"])
    import yaml

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
FORGE_BIN = os.path.abspath(os.environ.get("FORGE_BIN", "forge"))

# Each test scenario creates its own isolated FORGE_CONFIG dir so:
#   1. We know exactly where permissions.yaml lives.
#   2. Tests don't interfere with the real ~/.forge or ~/forge directories.
FORGE_CONFIG_DIR: str = ""  # set per-scenario

PASS = "\033[32mPASS\033[0m"
FAIL = "\033[31mFAIL\033[0m"

results = []


def perm_file() -> str:
    """Path to permissions.yaml inside the current test's FORGE_CONFIG dir."""
    return os.path.join(FORGE_CONFIG_DIR, "permissions.yaml")


def log(msg: str):
    print(f"  {msg}", flush=True)


def _format_permissions(perms: dict, label: str) -> None:
    """Print a labelled permissions block as indented YAML."""
    print(f"  {label}", flush=True)
    if not perms:
        print("    (empty — no permissions.yaml)", flush=True)
        return
    for line in yaml.dump(perms, default_flow_style=False, sort_keys=False).splitlines():
        print(f"    {line}", flush=True)


def log_permissions(before: dict, after: dict) -> None:
    """Print before/after permissions.yaml side by side (sequentially)."""
    print("  ┌─ before ─────────────────────────────────", flush=True)
    _format_permissions(before, "")
    print("  ├─ after ──────────────────────────────────", flush=True)
    _format_permissions(after, "")
    print("  └──────────────────────────────────────────", flush=True)


def assert_true(cond: bool, msg: str):
    if not cond:
        raise AssertionError(msg)


def read_permissions() -> dict:
    path = perm_file()
    if not os.path.exists(path):
        return {}
    with open(path) as f:
        return yaml.safe_load(f) or {}


def write_permissions(data: dict):
    path = perm_file()
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        yaml.dump(data, f, default_flow_style=False)


def remove_permissions():
    path = perm_file()
    if os.path.exists(path):
        os.remove(path)


def run_scenario(name: str, fn):
    print(f"\n{'─'*60}")
    print(f"Scenario: {name}")
    print(f"{'─'*60}")
    try:
        fn()
        print(f"Result: {PASS}")
        results.append((name, True, None))
    except Exception as e:
        print(f"Result: {FAIL} — {e}")
        results.append((name, False, str(e)))


def spawn_forge_with_prompt(tmpdir: str, forge_config: str, timeout: int = 30) -> pexpect.spawn:
    """
    Spawn `forge -p hello` inside tmpdir with a PTY.

    FORGE_CONFIG is set to an isolated temp dir so permissions.yaml is written
    there, not into the real user config directory.

    The MCP permission TUI guards on `stderr().is_terminal()` before rendering.
    pexpect.spawn() only allocates a PTY for stdout; stderr is a plain pipe so
    is_terminal() returns false and the prompt is skipped entirely.

    Fix: wrap the call in `sh -c 'forge ... 2>&1'` so both stdout and stderr
    share the same PTY file descriptor. With that, is_terminal() sees a TTY
    and the crossterm TUI renders and accepts keyboard input normally.
    """
    cmd = f"{FORGE_BIN} -p hello"
    env = {
        **os.environ,
        "TERM": "xterm-256color",
        "COLUMNS": "120",
        "LINES": "40",
        "FORGE_CONFIG": forge_config,
    }
    child = pexpect.spawn(
        "/bin/sh",
        args=["-c", f"exec {cmd} 2>&1"],
        cwd=tmpdir,
        timeout=timeout,
        encoding="utf-8",
        codec_errors="replace",
        env=env,
    )
    return child


def mcp_json(command: str, args=None) -> dict:
    server: dict = {"command": command}
    if args:
        server["args"] = args
    return {"mcpServers": {"test-server": server}}


# ---------------------------------------------------------------------------
# Scenario implementations
# ---------------------------------------------------------------------------

def make_dirs() -> "tuple[str, str]":
    """Return (tmpdir, forge_config) — two fresh isolated temp directories.

    forge_config is seeded with the real forge config files (toml, credentials,
    provider) so forge starts without a first-time setup prompt, but with no
    permissions.yaml so the MCP gate fires normally.
    """
    tmpdir = tempfile.mkdtemp(prefix="forge_mcp_cwd_")
    forge_config = tempfile.mkdtemp(prefix="forge_mcp_cfg_")

    # Copy config files from the real forge base dir so forge starts configured.
    real_base = None
    for candidate in [
        os.path.expanduser("~/forge"),
        os.path.expanduser("~/.forge"),
    ]:
        if os.path.isdir(candidate):
            real_base = candidate
            break

    if real_base:
        for name in [".forge.toml", ".config.json", ".credentials.json", "provider.json"]:
            src = os.path.join(real_base, name)
            if os.path.exists(src):
                shutil.copy2(src, os.path.join(forge_config, name))

    return tmpdir, forge_config


def scenario_accept_writes_allow_rule():
    """
    No permissions.yaml + local MCP server → prompt fires → user picks Accept
    → permissions.yaml written with allow rule for the server.
    """
    global FORGE_CONFIG_DIR
    tmpdir, forge_config = make_dirs()
    FORGE_CONFIG_DIR = forge_config
    try:
        # Write a local .mcp.json
        with open(os.path.join(tmpdir, ".mcp.json"), "w") as f:
            json.dump(mcp_json("echo", ["hello"]), f)

        before = read_permissions()
        log("Spawning forge (no permissions.yaml) ...")
        child = spawn_forge_with_prompt(tmpdir, forge_config)

        log("Waiting for permission prompt ...")
        child.expect("Allow MCP server", timeout=30)
        log("Prompt detected. Sending Enter (Accept) ...")
        child.send("\r")

        # Wait for forge to persist the decision
        time.sleep(4)
        child.close(force=True)

        after = read_permissions()
        log_permissions(before, after)
        policies = after.get("policies", [])
        assert_true(len(policies) > 0, "Expected at least one policy to be written")
        allow_policies = [p for p in policies if isinstance(p, dict) and p.get("permission") == "allow"]
        assert_true(len(allow_policies) > 0, "Expected an 'allow' policy to be written")
        mcp_rules = [p for p in allow_policies if isinstance(p.get("rule"), dict) and "mcp" in p["rule"]]
        assert_true(len(mcp_rules) > 0, "Expected an MCP rule in the allow policy")

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
        shutil.rmtree(forge_config, ignore_errors=True)


def scenario_reject_writes_deny_rule():
    """
    No permissions.yaml + local MCP server → prompt fires → user picks Reject
    → permissions.yaml written with deny rule for the server.
    """
    global FORGE_CONFIG_DIR
    tmpdir, forge_config = make_dirs()
    FORGE_CONFIG_DIR = forge_config
    try:
        with open(os.path.join(tmpdir, ".mcp.json"), "w") as f:
            json.dump(mcp_json("echo", ["hello"]), f)

        before = read_permissions()
        log("Spawning forge (no permissions.yaml) ...")
        child = spawn_forge_with_prompt(tmpdir, forge_config)

        log("Waiting for permission prompt ...")
        child.expect("Allow MCP server", timeout=30)
        log("Prompt detected. Sending Down then Enter (Reject) ...")
        # Send arrow-down as individual bytes to ensure raw-mode TUI receives it
        child.send("\x1b")
        time.sleep(0.1)
        child.send("[")
        time.sleep(0.1)
        child.send("B")
        time.sleep(0.5)
        child.send("\r")

        time.sleep(4)
        child.close(force=True)

        after = read_permissions()
        log_permissions(before, after)
        policies = after.get("policies", [])
        deny_policies = [p for p in policies if isinstance(p, dict) and p.get("permission") == "deny"]
        assert_true(len(deny_policies) > 0, "Expected a 'deny' policy to be written")
        mcp_rules = [p for p in deny_policies if isinstance(p.get("rule"), dict) and "mcp" in p["rule"]]
        assert_true(len(mcp_rules) > 0, "Expected an MCP deny rule for the server")

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
        shutil.rmtree(forge_config, ignore_errors=True)


def scenario_existing_allow_skips_prompt():
    """
    permissions.yaml already has an allow rule → forge starts without prompting.
    """
    global FORGE_CONFIG_DIR
    tmpdir, forge_config = make_dirs()
    FORGE_CONFIG_DIR = forge_config
    try:
        write_permissions({
            "policies": [
                {"permission": "allow", "rule": {"mcp": {"command": "echo"}}}
            ]
        })

        with open(os.path.join(tmpdir, ".mcp.json"), "w") as f:
            json.dump(mcp_json("echo", ["hello"]), f)

        before = read_permissions()
        log("Spawning forge (allow rule pre-written) ...")
        child = spawn_forge_with_prompt(tmpdir, forge_config)

        idx = child.expect(["Allow MCP server", pexpect.TIMEOUT, pexpect.EOF], timeout=15)
        child.close(force=True)

        after = read_permissions()
        log_permissions(before, after)
        assert_true(idx != 0, "Permission prompt appeared even though allow rule was pre-written")
        log("No permission prompt shown — correct.")

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
        shutil.rmtree(forge_config, ignore_errors=True)


def scenario_second_run_skips_prompt():
    """
    After accepting on first run (permissions.yaml written), a second forge
    invocation must NOT show the prompt again.
    """
    global FORGE_CONFIG_DIR
    tmpdir, forge_config = make_dirs()
    FORGE_CONFIG_DIR = forge_config
    try:
        with open(os.path.join(tmpdir, ".mcp.json"), "w") as f:
            json.dump(mcp_json("echo", ["hello"]), f)

        # First run — accept
        before_run1 = read_permissions()
        log("First run: accepting permission ...")
        child = spawn_forge_with_prompt(tmpdir, forge_config)
        child.expect("Allow MCP server", timeout=30)
        child.send("\r")
        time.sleep(4)
        child.close(force=True)

        after_run1 = read_permissions()
        log("  [run 1]")
        log_permissions(before_run1, after_run1)
        assert_true(os.path.exists(perm_file()), "permissions.yaml not created after first accept")

        # Second run — should NOT prompt
        before_run2 = read_permissions()
        log("Second run: verifying no prompt ...")
        child2 = spawn_forge_with_prompt(tmpdir, forge_config)
        idx = child2.expect(["Allow MCP server", pexpect.TIMEOUT, pexpect.EOF], timeout=15)
        child2.close(force=True)

        after_run2 = read_permissions()
        log("  [run 2]")
        log_permissions(before_run2, after_run2)
        assert_true(idx != 0, "Permission prompt appeared on second run — decision was not persisted")
        log("No prompt on second run — decision persisted correctly.")

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
        shutil.rmtree(forge_config, ignore_errors=True)


def scenario_custom_mcp_server_accept():
    """
    Add a custom realistic MCP server (npx-based filesystem server) to
    .mcp.json, start forge, answer Accept in the TUI prompt, and verify
    permissions.yaml contains an allow rule for that exact server.
    """
    global FORGE_CONFIG_DIR
    tmpdir, forge_config = make_dirs()
    FORGE_CONFIG_DIR = forge_config
    try:
        custom_server = {
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", tmpdir],
                }
            }
        }
        with open(os.path.join(tmpdir, ".mcp.json"), "w") as f:
            json.dump(custom_server, f)

        before = read_permissions()
        log("Spawning forge with custom filesystem MCP server ...")
        child = spawn_forge_with_prompt(tmpdir, forge_config)

        log("Waiting for permission prompt for 'filesystem' server ...")
        child.expect('Allow MCP server "filesystem"', timeout=30)
        log("Prompt detected. Sending Enter (Accept) ...")
        child.send("\r")

        time.sleep(4)
        child.close(force=True)

        after = read_permissions()
        log_permissions(before, after)
        policies = after.get("policies", [])
        allow_mcp = [
            p for p in policies
            if isinstance(p, dict) and p.get("permission") == "allow"
            and isinstance(p.get("rule"), dict) and "mcp" in p["rule"]
        ]
        assert_true(len(allow_mcp) > 0, "Expected an allow MCP rule in permissions.yaml")
        rule_mcp = allow_mcp[0]["rule"]["mcp"]
        assert_true(rule_mcp.get("command") == "npx", f"Expected command='npx' in rule, got: {rule_mcp}")

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
        shutil.rmtree(forge_config, ignore_errors=True)


def scenario_custom_mcp_server_reject():
    """
    Add a custom MCP server, start forge, answer Reject in the TUI prompt,
    and verify permissions.yaml contains a deny rule for that server.
    """
    global FORGE_CONFIG_DIR
    tmpdir, forge_config = make_dirs()
    FORGE_CONFIG_DIR = forge_config
    try:
        custom_server = {
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", tmpdir],
                }
            }
        }
        with open(os.path.join(tmpdir, ".mcp.json"), "w") as f:
            json.dump(custom_server, f)

        before = read_permissions()
        log("Spawning forge with custom filesystem MCP server ...")
        child = spawn_forge_with_prompt(tmpdir, forge_config)

        log("Waiting for permission prompt for 'filesystem' server ...")
        child.expect('Allow MCP server "filesystem"', timeout=30)
        log("Prompt detected. Sending Down + Enter (Reject) ...")
        # Send arrow-down as individual bytes to ensure raw-mode TUI receives it
        child.send("\x1b")
        time.sleep(0.1)
        child.send("[")
        time.sleep(0.1)
        child.send("B")
        time.sleep(0.5)
        child.send("\r")

        time.sleep(4)
        child.close(force=True)

        after = read_permissions()
        log_permissions(before, after)
        policies = after.get("policies", [])
        deny_mcp = [
            p for p in policies
            if isinstance(p, dict) and p.get("permission") == "deny"
            and isinstance(p.get("rule"), dict) and "mcp" in p["rule"]
        ]
        assert_true(len(deny_mcp) > 0, "Expected a deny MCP rule in permissions.yaml")
        rule_mcp = deny_mcp[0]["rule"]["mcp"]
        assert_true(rule_mcp.get("command") == "npx", f"Expected command='npx' in deny rule, got: {rule_mcp}")

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
        shutil.rmtree(forge_config, ignore_errors=True)


def scenario_user_scope_never_prompts():
    """
    A server in the user-scope config (~/.mcp.json relative to FORGE_CONFIG)
    must never prompt, regardless of permissions.yaml.

    The user-scope MCP file is `<FORGE_CONFIG>/.mcp.json`.
    """
    global FORGE_CONFIG_DIR
    tmpdir, forge_config = make_dirs()
    FORGE_CONFIG_DIR = forge_config
    try:
        # Write the server to the user-scope MCP path inside our isolated FORGE_CONFIG
        user_mcp = os.path.join(forge_config, ".mcp.json")
        with open(user_mcp, "w") as f:
            json.dump(mcp_json("echo", ["user-scope"]), f)

        before = read_permissions()
        log("Spawning forge (user-scope server, no permissions.yaml) ...")
        child = spawn_forge_with_prompt(tmpdir, forge_config)
        idx = child.expect(["Allow MCP server", pexpect.TIMEOUT, pexpect.EOF], timeout=15)
        child.close(force=True)

        after = read_permissions()
        log_permissions(before, after)
        assert_true(idx != 0, "Permission prompt appeared for a user-scope server — should be trusted unconditionally")
        log("No prompt for user-scope server — trusted unconditionally.")

    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
        shutil.rmtree(forge_config, ignore_errors=True)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    print("=" * 60)
    print("MCP Permission Policy — End-to-End Tests")
    print("=" * 60)
    print(f"  forge binary: {FORGE_BIN}")

    run_scenario("Accept → allow rule written to permissions.yaml", scenario_accept_writes_allow_rule)
    run_scenario("Reject → deny rule written to permissions.yaml", scenario_reject_writes_deny_rule)
    run_scenario("Pre-existing allow rule → no prompt", scenario_existing_allow_skips_prompt)
    run_scenario("Second run after accept → no prompt", scenario_second_run_skips_prompt)
    run_scenario("Custom MCP server (npx) Accept → allow rule in permissions.yaml", scenario_custom_mcp_server_accept)
    run_scenario("Custom MCP server (npx) Reject → deny rule in permissions.yaml", scenario_custom_mcp_server_reject)
    run_scenario("User-scope server → never prompts", scenario_user_scope_never_prompts)

    # Summary
    print(f"\n{'='*60}")
    print("SUMMARY")
    print(f"{'='*60}")
    passed = sum(1 for _, ok, _ in results if ok)
    failed = sum(1 for _, ok, _ in results if not ok)
    for name, ok, err in results:
        status = PASS if ok else FAIL
        print(f"  {status}  {name}")
        if err:
            for line in textwrap.wrap(err, width=72):
                print(f"         {line}")
    print(f"\n{passed}/{len(results)} passed")
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
