---
name: write-release-notes
description: Generate engaging, high-energy release notes for a given version tag. Fetches the release from GitHub, retrieves every linked PR's title and description, then synthesizes all changes into a polished, user-facing release note with an enthusiastic tone. Use when the user asks to write, generate, or create release notes for a version (e.g. "write release notes for v1.32.0", "generate release notes for the latest release", "create changelog for v2.0").
---

# Write Release Notes

Generate clear, informative, and enthusiastic release notes by pulling live data from GitHub and synthesizing every PR into a cohesive narrative.

## Workflow

### 1. Fetch Release Data

Run the bundled script to pull the release metadata and all linked PR details in one shot:

```bash
bash .forge/skills/write-release-notes/scripts/fetch-release-data.sh <version> [owner/repo]
```

- `<version>`: The release tag (e.g. `v1.32.0`)
- `[owner/repo]`: Optional. Defaults to the current repo detected via `gh repo view`.

The script outputs two sections:
- `### RELEASE METADATA ###` — tag name, publish date, release name, raw body
- `### PR DETAILS ###` — one JSON object per PR with: `number`, `title`, `body`, `labels`, `author`, `mergedAt`, `url`

### 2. Categorize Changes

Group PRs by their conventional commit prefix or label:

| Category | Prefixes / Labels |
|---|---|
| Features | `feat`, `type: feature` |
| Bug Fixes | `fix`, `type: fix` |
| Performance | `perf` |
| Refactors | `refactor` |
| Maintenance | `chore`, `docs`, `ci`, `build`, `deps` |

Dependency bumps (e.g. Dependabot PRs) go into Maintenance. Skip PRs with `error: "not found"`.

### 3. Write the Release Notes

Produce a Markdown document with the following structure. Keep the tone **informative and enthusiastic** — explain what changed and why it matters, without resorting to marketing fluff.

```markdown
# [Product Name] [Version] — [Descriptive Tagline]

> One-sentence summary of what this release focuses on.

## What's New

[2-4 sentence narrative covering the biggest features and fixes. 
Describe what changed and what users can now do. Use active voice. Be factual but upbeat.]

## Highlights

### [Feature/Fix Category]
**[PR Title rephrased as a clear description of the change]**
[1-2 sentences expanding on the PR description. Explain what changed and what users can now do differently. 
If the PR body has useful context, distill it. If empty, infer from the title.]

[Repeat for each significant PR — skip pure chores/dep bumps unless noteworthy]

## Bug Fixes & Reliability

[Bullet list of fixes, each with a brief impact statement]

## Under the Hood

[Brief paragraph or bullet list covering refactors, maintenance, and dep updates — 
keep it light, acknowledge the work without boring the reader]

## Contributors

A huge thank you to everyone who made this release happen: [list @handles — exclude bots like @dependabot]

---
**Full changelog**: [GitHub Release link]
```

### 4. Tone & Style Guidelines

- **Lead with what changed**: "You can now..." or "Forge now..." beats "We added..."
- **Be specific**: Name the feature and describe what it does, not just the category
- **Be informative, not marketty**: Avoid vague adjectives like "seamless", "smarter", "blazing", "powerful", "rock-solid". Instead, state the concrete fact (e.g. "editor no longer spawns a git process on every keystroke" beats "blazing-fast editor")
- **Enthusiasm through substance**: Let the actual improvement speak for itself. Use active, direct language.
- **Short paragraphs**: Max 3 sentences per block
- **Skip internal jargon**: Translate crate names and internal concepts into plain language
- **Celebrate contributors**: Name them by handle
- **Tagline formula**: `[Version] — [Factual Theme Description]` (e.g. "v1.32.0 — Terminal Context, File Drop Support, Windows Performance")
- **No implementation details**: Do not mention internal module names, struct names, function names, crate names, or how something was implemented. Focus purely on what the user experiences or gains.
- **No PR/issue references**: Do not include PR numbers, issue numbers, or links to GitHub PRs/issues in the release notes. Focus on the changes themselves, not their tracking identifiers.

### 5. Contributors Filter

Only include **external contributors** in the Contributors section — exclude the core team:
- `@tusharmath`
- `@amitksingh1490`
- `@laststylebender14`
- Bots (e.g. `@dependabot`)

If no external contributors exist, omit the Contributors section entirely.

### 6. Validate Length

After writing the release notes, run the bundled validation script to confirm the output is under 2000 characters:

```bash
echo "<release notes>" | bash .forge/skills/write-release-notes/scripts/validate-release-notes.sh
```

If it prints `FAIL`, trim the draft and re-run until it prints `PASS`:
- Remove the Under the Hood section first
- Consolidate Bug Fixes into a shorter bullet list
- Shorten individual PR descriptions to one tight sentence
- Remove the least impactful Highlights entries

### 7. Output

Print the final release notes directly in the chat. Do not write to a file unless the user explicitly asks.

## Notes

- The script handles ANSI color codes injected by `gh` CLI automatically.
- PRs not found (closed without merge, private, etc.) are silently skipped.
- If the release has no linked PRs in its body, fall back to listing commits between tags:
  ```bash
  gh api repos/<owner>/<repo>/compare/<prev_tag>...<version> --jq '.commits[].commit.message'
  ```
