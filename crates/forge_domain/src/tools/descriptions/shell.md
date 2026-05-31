Executes shell commands. The `cwd` parameter sets the working directory for command execution. If not specified, defaults to `{{env.cwd}}`.

CRITICAL: Do NOT use `cd` commands in the command string. This is FORBIDDEN. Always use the `cwd` parameter to set the working directory instead. Any use of `cd` in the command is redundant, incorrect, and violates the tool contract.

IMPORTANT: This tool is for terminal operations like git, npm, docker, etc. DO NOT use it for file operations (reading, writing, editing, searching, finding files) - use the specialized tools for this instead.

Always quote file paths that contain spaces with double quotes.

Usage notes:
  - The command argument is required.
  - It is very helpful if you write a clear, concise description of what this command does in 5-10 words.
  - If the output exceeds {{config.stdoutMaxPrefixLength}} prefix lines or {{config.stdoutMaxSuffixLength}} suffix lines, or if a line exceeds {{config.stdoutMaxLineLength}} characters, it will be truncated and the full output will be written to a temporary file. You can use read with start_line/end_line to read specific sections or fs_search to search the full content. Because of this, you should NOT use `head`, `tail`, or other truncation commands to limit output - just run the command directly.
  - Prefer dedicated tools over shell for file operations: `{{tool_names.fs_search}}` for search, `{{tool_names.read}}` for reading, `{{tool_names.patch}}` for editing, `{{tool_names.write}}` for writing.
  - When issuing multiple commands:
    - If the commands are independent and can run in parallel, make multiple `{{tool_names.shell}}` tool calls in a single message. For example, if you need to run "git status" and "git diff", send a single message with two `{{tool_names.shell}}` tool calls in parallel.
    - If the commands depend on each other and must run sequentially, use a single `{{tool_names.shell}}` call with '&&' to chain them together (e.g., `git add . && git commit -m "message" && git push`). For instance, if one operation must complete before another starts (like mkdir before cp, write before shell for git operations, or git add before git commit), run these operations sequentially instead.
    - Use ';' only when you need to run commands sequentially but don't care if earlier commands fail
    - DO NOT use newlines to separate commands (newlines are ok in quoted strings)
  - DO NOT use `cd <directory> && <command>`. Use the `cwd` parameter to change directories instead.

Good examples:
  - With explicit cwd: cwd="/foo/bar" with command: pytest tests

Bad example:
  cd /foo/bar && pytest tests

Returns complete output including stdout, stderr, and exit code for diagnostic purposes.

Background mode (background=true):
  - Use for starting long-running services (web servers, databases, VMs) that must keep running after the tool call returns.
  - The command is launched via `nohup` in the background. The tool returns immediately with the process PID and a log file path.
  - After a brief delay the tool verifies the process is still running. If it exited early, the log output is returned for diagnosis.
  - You can later inspect the background process log with the `read` tool, or check if the process is alive with `kill -0 <PID>`.
  - Example: background=true with command: python3 -m http.server 8080 --directory /var/www/html