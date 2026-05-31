A powerful search tool built on ripgrep

Usage:
- ALWAYS use `{{tool_names.fs_search}}` for search tasks. NEVER invoke `grep` or `rg` as a Bash command. The `{{tool_names.fs_search}}` tool has been optimized for correct permissions and access.
- Supports full regex syntax (e.g., "log.*Error", "function\\s+\\w+")
- Filter files with glob parameter (e.g., "*.js", "**/*.tsx") or type parameter (e.g., "js", "py", "rust")
- Output modes: "content" shows matching lines, "files_with_matches" shows only file paths (default), "count" shows match counts
- Use Task tool for open-ended searches requiring multiple rounds
- Pattern syntax: Uses ripgrep (not grep) - literal braces need escaping (use `interface\\{\\}` to find `interface{}` in Go code)
- Multiline matching: By default patterns match within single lines only. For cross-line patterns like `struct \\{[\\s\\S]*?field`, use `multiline: true`
- Truncated results: When search results exceed the size limit, the output will include a WARNING indicating results are INCOMPLETE and a `full_output` path to a temporary file containing the complete untruncated results. You MUST read this file or refine your search pattern to ensure full coverage. Never assume truncated results represent all matches.
