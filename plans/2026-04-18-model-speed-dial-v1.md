# Model Speed-Dial: `:1`, `:2`, `:3` … Quick-Switch Slots

## Objective

Introduce a "speed-dial" feature that lets a user pre-bind frequently used
models (e.g. `claude-opus`, `claude-sonnet`, `gpt-5.4`) to single-digit slots
and instantly switch the **session model** with a one-keystroke command —
`:1`, `:2`, `:3` … from the zsh shell plugin (and `/1`, `/2`, `/3` … from
inside the interactive `forge` TUI).

Switching a slot must reuse the existing **session-only model override**
plumbing (`_FORGE_SESSION_MODEL` / `_FORGE_SESSION_PROVIDER` →
`FORGE_SESSION__MODEL_ID` / `FORGE_SESSION__PROVIDER_ID`) so the global
config is untouched and `:cr` (config-reload) still resets cleanly. Slots
themselves are persisted in the global forge config so they survive shells.

## Assumptions

- Slots are single decimal digits `1`–`9` (nine slots is more than enough;
  `0` is reserved as the "reset to global" slot, mirroring `:cr`).
- Slot bindings live in the global config TOML (resolved via
  `forge config path`) under a new `[speed_dial]` table:
  `speed_dial.<slot> = { provider = "<provider_id>", model = "<model_id>" }`.
- `:N` (or `/N`) **with no argument** switches the session model to slot N
  and prints a success line; `:N <prompt>` switches and then forwards the
  prompt to the active agent (same flow as `:<agent> <prompt>`).
- Setup uses the already-familiar interactive picker. A new
  `:speed-dial` (alias `:sd`) command opens an fzf chooser of slots, then
  the existing `_forge_pick_model` to assign one. Direct CLI form is also
  supported: `:sd <N>` (assign current session/global model to slot N) and
  `forge config set speed-dial <N> <provider> <model>`.
- Backwards compatible: an unset slot prints a friendly error suggesting
  `:sd <N>` and is a no-op; nothing else in the plugin or TUI changes
  behaviour.
- Must work with the zsh plugin's `:command` regex, which **today rejects
  digit-leading tokens** (`shell-plugin/lib/dispatcher.zsh:99`). The regex
  must be widened.

## Implementation Plan

### 1. Domain & persistence layer (`crates/forge_domain`, `crates/forge_app`)

- [ ] Task 1. Add a `SpeedDialEntry { provider: ProviderId, model: ModelId }`
      domain type (with `derive_setters`, `serde`) and a
      `SpeedDial(BTreeMap<u8, SpeedDialEntry>)` newtype keyed by slot number
      `1..=9`. Place next to the existing model/provider config types.
      Rationale: `BTreeMap` gives stable ordering for listing in `:info`
      and serialises to a TOML table cleanly.
- [ ] Task 2. Extend the global config struct (the same struct that
      currently holds `model`, `provider`, `commit`, `suggest`,
      `reasoning_effort`) with an optional `speed_dial: Option<SpeedDial>`
      field defaulting to empty so existing configs continue to load.
- [ ] Task 3. Add `forge.schema.json` regeneration entry for the new field
      (the project already maintains this generated schema).

### 2. CLI surface (`crates/forge_main` config subcommands)

- [ ] Task 4. Extend the `forge config get` subcommand to support
      `speed-dial` (prints all slots in `slot<TAB>provider<TAB>model`
      porcelain form) and `speed-dial <N>` (prints two lines — provider
      then model — mirroring `config get commit`).
- [ ] Task 5. Extend `forge config set` to support
      `speed-dial <N> <provider_id> <model_id>` and
      `speed-dial <N> --clear` (removes the binding). Validate `N ∈ 1..=9`
      and that `(provider, model)` exists in the provider registry, reusing
      the validation done by `config set model`.
- [ ] Task 6. Add a `forge config get speed-dial-slot <N>` helper command
      that prints `provider_id<TAB>model_id` on a single line — used by the
      shell plugin to resolve a slot into env-var values without fragile
      TOML parsing in zsh.

### 3. In-TUI slash commands (`crates/forge_main/src/model.rs`,
   `built_in_commands.json`, `ui.rs`)

- [ ] Task 7. Add `SlashCommand::SpeedDial { slot: u8, message:
      Option<String> }` and parse `/1`–`/9` (optionally followed by a
      prompt) in `ForgeCommandManager::parse`
      (`crates/forge_main/src/model.rs:237`). Make sure the parser still
      treats unrecognised `/<digits>` (e.g. `/10`) as message text.
- [ ] Task 8. Add a `SlashCommand::SpeedDialManage` variant for `/speed-dial`
      (alias `/sd`) that opens an interactive picker (slot list → model
      picker), reusing the existing model-selection UI used by
      `/config-model` (`UI::on_show_commands` and the model picker hooked
      into `ui.rs:415`). Handler updates the in-process session model and
      writes the binding back to the global config via the new
      `config set speed-dial` plumbing from Task 5.
- [ ] Task 9. Implement the `/N` handler: look up slot N from config; if
      missing, print a hint; otherwise apply the same session-override
      effect that `/model` applies (set the in-memory session model +
      provider). If a `message` is present, dispatch it to the active
      agent immediately, mirroring `:<agent> <prompt>` semantics.
- [ ] Task 10. Register entries in `crates/forge_main/src/built_in_commands.json`
      so completion lists them:
      - `{"command": "speed-dial", "description": "Manage model speed-dial slots [alias: sd]"}`
      - `{"command": "1", "description": "Switch to speed-dial slot 1"}`
      - … through slot `9`.
      Generation may be done at build time (a small `build.rs` or static
      array in `default_commands`) to avoid manual repetition.

### 4. Zsh plugin: dispatcher, action, completion
   (`shell-plugin/`)

- [ ] Task 11. **Critical compatibility fix.** Widen the accept-line regex
      at `shell-plugin/lib/dispatcher.zsh:99` from
      `^:([a-zA-Z][a-zA-Z0-9_-]*)( (.*))?$` to also allow a single
      digit `1`–`9` as a complete token, e.g.
      `^:([a-zA-Z][a-zA-Z0-9_-]*|[1-9])( (.*))?$`.
      Without this change `:1` falls through to `zle accept-line` and
      becomes a literal shell command. This is the **only** plugin-level
      change required to make speed-dial trigger; all other behaviour is
      additive.
- [ ] Task 12. Add a new dispatch case before the catch-all in
      `dispatcher.zsh:144` that matches `[1-9]` and invokes
      `_forge_action_speed_dial "$user_action" "$input_text"`.
      Also add `speed-dial|sd` → `_forge_action_speed_dial_manage`.
- [ ] Task 13. Implement `_forge_action_speed_dial` in
      `shell-plugin/lib/actions/config.zsh`:
      1. Resolve the slot via
         `$_FORGE_BIN config get speed-dial-slot "$slot"`.
      2. If empty, log an error suggesting `:sd $slot` and return.
      3. Parse `provider_id<TAB>model_id`, then set
         `_FORGE_SESSION_MODEL` and `_FORGE_SESSION_PROVIDER` exactly the
         way `_forge_action_session_model` does
         (`shell-plugin/lib/actions/config.zsh:345-346`).
      4. Print a success line including the slot number, model id, and
         provider id.
      5. If `$input_text` is non-empty, fall through to the same prompt
         dispatch path as the default action
         (`_forge_exec_interactive -p "$input_text" --cid …`), so
         `:2 explain this diff` works as a one-shot.
- [ ] Task 14. Implement `_forge_action_speed_dial_manage`:
      - With **no argument**: open fzf showing slots `1`–`9`, the bound
        model (or `<empty>`) for each; on selection, reuse
        `_forge_pick_model` to pick a model, then call
        `$_FORGE_BIN config set speed-dial <N> <provider_id> <model_id>`.
      - With `<N>` argument: open `_forge_pick_model` directly for that
        slot.
      - With `<N> --clear`: call `config set speed-dial <N> --clear`.
- [ ] Task 15. Update `shell-plugin/lib/completion.zsh` (and any helper that
      builds the command list from `forge show-commands`) so the slot
      commands and `speed-dial`/`sd` show up in completion. Because
      Task 10 surfaces them through the canonical command registry, this
      should be automatic — verify only.
- [ ] Task 16. Add a section to `shell-plugin/keyboard.zsh` /
      `:keyboard-shortcuts` output describing the new slots.

### 5. Visibility & docs

- [ ] Task 17. Surface active speed-dial bindings in `:info`
      (`crates/forge_main/src/info.rs`) — a small "Speed Dial" section
      listing each populated slot. This makes the feature discoverable.
- [ ] Task 18. Add a "Model Speed Dial" subsection to `README.md`
      (around the existing model commands at `README.md:319`) showing the
      configuration TOML, the `:sd` setup flow, and the `:1` `:2` `:3`
      usage. Include the requested concrete example (slot 1 →
      `claude-opus`, slot 2 → `claude-sonnet`, slot 3 → `gpt-5.4`).
- [ ] Task 19. Add a sample `[speed_dial]` block to any shipped example
      config under `templates/` (only if such a file already exists; do
      not create new docs).

### 6. Tests

- [ ] Task 20. Unit tests in `forge_domain` covering: `SpeedDial` serde
      round-trip, slot range validation (1..=9 only), TOML round-trip
      with and without the field present (backwards compat).
- [ ] Task 21. Unit tests in `forge_main::model` covering parsing of
      `/1` … `/9`, `/1 some prompt`, and the negative case `/10` (must
      remain a literal message).
- [ ] Task 22. Integration test for `forge config set/get speed-dial`
      using the existing test harness for the `config` subcommand.
- [ ] Task 23. Snapshot test (`cargo insta`) for the `:info` output that
      includes a populated speed-dial section.
- [ ] Task 24. Add a small zsh test (under `shell-plugin/` if there is
      existing test scaffolding; otherwise document the manual smoke test
      in the PR description) that asserts the widened dispatcher regex
      matches `:1`, `:9 hello world`, and still rejects `:10abc`.

## Verification Criteria

- Running `:sd 1` opens fzf, picking `claude-opus` writes
  `speed_dial.1 = { provider = "...", model = "claude-opus-..." }` to the
  resolved config file (`forge config path`).
- After binding slots 1/2/3, `:1` switches the session model to
  claude-opus and `forge config get model` still returns the
  globally-configured model (proving session scope).
- `:1 explain this repo` switches the model **and** sends the prompt in a
  single command, with output rendered by the chosen model.
- `:cr` (config-reload) still clears the override set by `:1`, returning
  to global config.
- Inside `forge` TUI, typing `/1` produces the same effect as `:1`
  outside it.
- `:info` shows a "Speed Dial" block enumerating populated slots.
- All existing tests, `cargo insta test --accept`, and `cargo check`
  succeed.

## Potential Risks and Mitigations

1. **Regex widening breaks an existing user shell habit** (e.g. someone
   who literally types `:1` as a typo today and expects it to remain a
   shell error). Mitigation: only match the closed set `[1-9]` (single
   digit, no suffix), so any other digit-leading input still falls
   through to `zle accept-line`.
2. **Slot collision with future named commands.** Mitigation: numeric
   slots live in their own namespace; reserve `0` for "reset" and
   document that future commands will not start with a digit.
3. **TUI parser ambiguity** between `/1` (slot) and `/<message starting
   with 1>`. Mitigation: only match a leading slash followed by **exactly
   one digit `1`–`9`** and either end-of-input or a space; everything
   else goes to `SlashCommand::Message`.
4. **Config schema drift** — old configs without `[speed_dial]` must keep
   loading. Mitigation: field is `Option<SpeedDial>` with `#[serde(default)]`
   and tested in Task 20.
5. **Provider/model id rename or removal** leaves dangling slot bindings.
   Mitigation: validate at switch time; if the bound model is no longer
   known, print a helpful error and leave the session unchanged (do not
   silently fall back).
6. **Completion noise** — adding nine new commands could clutter
   completion. Mitigation: tag slot commands with a distinct
   `description` prefix (`[slot]`) so they group visually; consider
   filtering them out of completion when no slot is bound (optional
   polish).

## Alternative Approaches

1. **Pure-shell implementation, no Rust changes.** Store slot bindings in
   a zsh-specific file (`~/.config/forge/speed_dial.zsh`) sourced by the
   plugin; `:1` simply sets env vars locally. Trade-off: zero Rust work
   and zero TUI integration — but bindings would not be shared with the
   `forge` TUI, with `:info`, or with future GUI front-ends, and we'd
   re-implement TOML parsing in zsh. Rejected as the primary path.
2. **Bind slots to keyboard shortcuts (`Alt+1`, `Alt+2`, …) via ZLE
   widgets** instead of `:N` text commands. Trade-off: even faster (one
   keystroke), but invisible in `show-commands`/completion, harder to
   document, and does not work inside the TUI. Could be added later as a
   complement to the `:N` commands proposed here.
3. **Re-use the existing `[agents]` mechanism**, treating each speed-dial
   entry as a synthetic agent. Trade-off: leverages an existing surface,
   but conflates "agent" (prompt + tools + model) with "model only" and
   would inflate the agent picker. Rejected for separation of concerns.

## Manual Smoke

No automated zsh test scaffolding exists in `shell-plugin/` today. Until one
is added, these are the manual steps to smoke-test the speed-dial feature.

### Regex-level checks (no forge binary required)

```zsh
# From a fresh zsh, source only the dispatcher fragment:
source shell-plugin/lib/dispatcher.zsh 2>/dev/null || true

test_accept() {
  local buf="$1"
  if [[ "$buf" =~ "^:([a-zA-Z][a-zA-Z0-9_-]*|[1-9])( (.*))?$" ]]; then
    print -- "MATCH  user=${match[1]}  params=${match[3]}  <- $buf"
  else
    print -- "REJECT <- $buf"
  fi
}

test_accept ':1'                 # expect: MATCH user=1 params=
test_accept ':9 hello world'     # expect: MATCH user=9 params=hello world
test_accept ':10'                # expect: REJECT
test_accept ':10abc'             # expect: REJECT
test_accept ':1abc'              # expect: REJECT
test_accept ':0'                 # expect: REJECT
test_accept ':model opus'        # expect: MATCH user=model params=opus
test_accept ':sd 3 --clear'      # expect: MATCH user=sd params=3 --clear
```

### End-to-end (with the forge binary installed)

1. `forge config set speed-dial 1 Anthropic claude-opus-4-20250514`
2. `forge config set speed-dial 2 Anthropic claude-sonnet-4-20250514`
3. `forge config get speed-dial` — expect both slots in porcelain form.
4. `forge config get speed-dial-slot 1` — expect `Anthropic<TAB>claude-opus-4-20250514`.
5. Start a zsh with the plugin sourced. Type `:1` — expect a "Speed-dial 1 → claude-opus-…" log line.
6. Type `:cr` — expect "Session overrides cleared".
7. Type `:1 hello world` — expect the session switch line AND the prompt to dispatch.
8. Type `:10` — expect the normal shell "command not found" (dispatcher regex should reject it).
9. Type `:sd` — expect an fzf chooser of slots 1..9.
10. Type `:sd 2 --clear`; then `forge config get speed-dial 2` should return empty / exit non-zero.
11. Launch `forge` (TUI); type `/1` — expect the same slot-1 behaviour as `:1` outside.
12. Type `:info` — expect a "Speed Dial" block listing slot 1.

Any deviation is a regression and should be filed before merging.
