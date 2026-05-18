---
name: test-mcp-permissions
description: Test the MCP server permission policy feature end-to-end. Use when asked to test MCP permissions, verify that local MCP servers are gated by policy, or validate the allow/deny/prompt behavior for MCP connections introduced in PR #3324.
---

# Test MCP Permissions

This skill validates the MCP server permission policy feature (PR #3324). The feature gates **local-scope** MCP servers (`.mcp.json` in the project directory) through a permission prompt at startup, while **user-scope** servers (`~/.forge/.mcp.json`) are trusted unconditionally.

## How the Feature Works

1. **Startup flow**: `UI::request_local_mcp_permissions()` reads the local `.mcp.json` and calls `McpApp::request_mcp_permissions(cfg)` **before** the REPL starts, so the prompt never races with user input.
2. **Permission check**: For each enabled local server, a `PermissionOperation::Mcp` is evaluated against `~/.forge/permissions.yaml` by `PolicyEngine`.
3. **Policy result**:
   - `Allow` → server connects silently
   - `Deny` → server is filtered out silently
   - `Confirm` (no matching rule) → user is prompted
4. **Prompt**: A two-choice `ConfirmPermission` (Accept / Reject) is shown with the server's command/url as a header. **Both** choices are persisted — the user is never asked again for the same server+cwd combination.
5. **Import shortcut**: `/mcp import` auto-persists `Allow` via `allow_mcp_servers()` — importing itself counts as consent, no prompt shown.

## Permissions File

`~/.forge/permissions.yaml` — written decisions look like:

```yaml
# stdio server (Allow)
policies:
  - permission: allow
    rule:
      mcp:
        command: npx
        args: ["-y", "@github/mcp"]
        dir: /path/to/project

# HTTP server (Deny)
  - permission: deny
    rule:
      mcp:
        url: "https://untrusted.example.com/sse"
        dir: /path/to/project
```

Glob patterns work in all fields (`command: "np*"`, `url: "https://trusted.com/*"`).

## Test Scenarios

### Scenario 1 — No permissions.yaml: prompt fires

```bash
rm -f ~/.forge/permissions.yaml
# Add a local MCP server to .mcp.json in the project dir:
echo '{"mcpServers":{"test-server":{"command":"npx","args":["-y","@github/mcp"]}}}' > .mcp.json
forge
```

**Expected:** Prompt appears — `Allow MCP server "test-server" to connect?` with `command: npx` shown as a header line. Choose **Accept**.

**Verify:**
```bash
cat ~/.forge/permissions.yaml
# → contains: permission: allow, mcp: {command: npx, args: ["-y", "@github/mcp"], dir: <cwd>}
```

---

### Scenario 2 — Accept persisted: no prompt on second run

After Scenario 1 (accepted), restart forge in the same directory.

**Expected:** No prompt. Server connects silently.

---

### Scenario 3 — Reject persisted: server silently blocked

Run Scenario 1 again (`rm ~/.forge/permissions.yaml`, restart forge), choose **Reject**.

**Expected:** Forge starts without the server's tools available.

**Verify:**
```bash
cat ~/.forge/permissions.yaml
# → contains: permission: deny
```

Ask forge to use a tool from that server — it should report it as unavailable.

---

### Scenario 4 — User-scope server: never prompted

Add a server to `~/.forge/.mcp.json` (user scope, not `.mcp.json` in cwd).

```bash
rm -f ~/.forge/permissions.yaml
forge
```

**Expected:** No prompt. User-scope servers bypass the permission gate and always connect.

---

### Scenario 5 — `mcp import` auto-approves

```bash
rm -f ~/.forge/permissions.yaml
forge
# Inside forge REPL:
/mcp import
```

**Expected:** No permission prompt during import. After import, `~/.forge/permissions.yaml` contains `allow` rules for each imported server.

---

### Scenario 6 — Glob rule pre-set in permissions.yaml

```bash
cat > ~/.forge/permissions.yaml << 'EOF'
policies:
  - permission: allow
    rule:
      mcp:
        command: "np*"
EOF
forge
```

**Expected:** No prompt for any stdio server whose command starts with `np` (e.g. `npx`). The glob match skips the prompt entirely.

---

### Scenario 7 — HTTP MCP server

```bash
rm -f ~/.forge/permissions.yaml
echo '{"mcpServers":{"http-server":{"url":"https://mcp.example.com/sse"}}}' > .mcp.json
forge
```

**Expected:** Prompt shows `url: https://mcp.example.com/sse` as header. Accepting writes:
```yaml
- permission: allow
  rule:
    mcp:
      url: "https://mcp.example.com/sse"
      dir: <cwd>
```

---

## Quick Reset Between Tests

```bash
rm -f ~/.forge/permissions.yaml
rm -f .mcp.json
```

## Key Code Locations

| What | File |
|---|---|
| Startup permission gate | `crates/forge_main/src/ui.rs:445-457` |
| McpApp orchestration | `crates/forge_app/src/mcp_app.rs` |
| Policy prompt logic | `crates/forge_services/src/policy.rs:218-244` |
| MCP rule matching | `crates/forge_domain/src/policies/rule.rs:111-116` |
| MCP filter (glob match) | `crates/forge_domain/src/policies/rule.rs:159-181` |
| Default permissions | `crates/forge_services/src/permissions.default.yaml` |
