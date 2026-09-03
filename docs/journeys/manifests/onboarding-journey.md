journey: onboarding-journey
title: "Onboarding: install to first tool call"
fr: [FR-001, FR-004, FR-009, FR-011]
steps:
  - step: install binary
    command: curl -sSfL https://github.com/KooshaPari/forgecode/releases/latest/download/install.sh | sh
    expect: binary installed to /usr/local/bin/forge-dev (or ~/.local/bin/forge-dev when /usr/local/bin is not writable); exit code 0
    verify: which forge-dev

  - step: verify version
    command: forge-dev --version
    expect: prints a version string >= 2.9.9 and exits 0
    verify: forge-dev --version | grep -E "^[0-9]+\.[0-9]+\.[0-9]+"

  - step: first session (single-shot)
    command: forge-dev -p "Reply with exactly: ready"
    expect: agent response contains "ready"; exit code 0; no interactive hang (stdin is not a TTY)
    verify: forge-dev -p "Reply with exactly: ready" 2>&1 | grep -q "ready"

  - step: configure provider credentials
    command: forge-dev provider --help
    expect: provider command group listed with auth subcommands; credentials are stored locally at ~/.forge/.credentials.json with 0o600 permissions
    verify: ls -l ~/.forge/.credentials.json 2>/dev/null | grep -E "^-rw-------|^-rw-r-----"

  - step: custom rules in forge.yaml
    command: echo "custom_rules: |\n  1. Answer in one sentence.\n" > forge.yaml && forge-dev -p "What is 2+2? Answer per project rules."
    expect: session loads forge.yaml custom_rules and the response follows the rule (one sentence)
    verify: forge-dev -p "What is 2+2? Answer per project rules." 2>&1 | grep -q "4"

  - step: first tool call (shell command via CLI)
    command: forge-dev -p "Run the shell command: echo hello-tool-call"
    expect: agent invokes the shell tool and reports output "hello-tool-call"
    verify: forge-dev -p "Run the shell command: echo hello-tool-call" 2>&1 | grep -q "hello-tool-call"

  - step: zsh setup (optional, zsh only)
    command: forge-dev setup
    expect: .zshrc updated with plugin + theme lines; idempotent on re-run
    verify: grep -q "forge.plugin.zsh" ~/.zshrc
