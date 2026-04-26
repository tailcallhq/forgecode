Executes shell commands. The `cwd` parameter sets the working directory for command execution. If not specified, defaults to `{{env.cwd}}`.

CRITICAL: Do NOT use `cd` commands in the command string. This is FORBIDDEN. Always use the `cwd` parameter to set the working directory instead. Any use of `cd` in the command is redundant, incorrect, and violates the tool contract.

IMPORTANT: This tool is for terminal operations like git, npm, docker, etc. DO NOT use it for file operations (reading, writing, editing, searching, finding files) - use the specialized tools for this instead.

Before executing the command, please follow these steps:

1. Directory Verification:
   - If the command will create new directories or files, first use `shell` with `ls` to verify the parent directory exists and is the correct location
   - For example, before running "mkdir foo/bar", first use `ls foo` to check that "foo" exists and is the intended parent directory

2. Command Execution:
   - Always quote file paths that contain spaces with double quotes (e.g., python "path with spaces/script.py")
   - Examples of proper quoting:
     - mkdir "/Users/name/My Documents" (correct)
     - mkdir /Users/name/My Documents (incorrect - will fail)
     - python "/path/with spaces/script.py" (correct)
     - python /path/with spaces/script.py (incorrect - will fail)
   - After ensuring proper quoting, execute the command.
   - Capture the output of the command.

Usage notes:
  - The command argument is required.
  - It is very helpful if you write a clear, concise description of what this command does in 5-10 words.
  - If the output exceeds {{config.stdoutMaxPrefixLength}} prefix lines or {{config.stdoutMaxSuffixLength}} suffix lines, or if a line exceeds {{config.stdoutMaxLineLength}} characters, it will be truncated and the full output will be written to a temporary file. You can use read with start_line/end_line to read specific sections or fs_search to search the full content. Because of this, you should NOT use `head`, `tail`, or other truncation commands to limit output - just run the command directly.
  - Do not use {{tool_names.shell}} with the `find`, `grep`, `cat`, `head`, `tail`, `sed`, `awk`, or `echo` commands, unless explicitly instructed or when these commands are truly necessary for the task. Instead, always prefer using the dedicated tools for these commands:
    - File search: Use `{{tool_names.fs_search}}` (NOT find or ls)
    - Content search: Use `{{tool_names.fs_search}}` with regex (NOT grep or rg)
    - Read files: Use `{{tool_names.read}}` (NOT cat/head/tail)
    - Edit files: Use `{{tool_names.patch}}`(NOT sed/awk)
    - Write files: Use `{{tool_names.write}}` (NOT echo >/cat <<EOF)
    - Communication: Output text directly (NOT echo/printf)
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