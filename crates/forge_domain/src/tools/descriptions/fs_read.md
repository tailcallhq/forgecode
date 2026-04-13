Reads a file from the local filesystem. You can access any file directly by using this tool. Assume this tool is able to read all files on the machine. If the User provides a path to a file assume that path is valid. It is okay to read a file that does not exist; an error will be returned.

Usage:
- The file_path parameter must be an absolute path, not a relative path
- By default, it reads up to {{config.maxReadSize}} lines starting from the beginning of the file
- You can optionally specify a line start_line and end_line (especially handy for long files), but it's recommended to read the whole file by not providing these parameters
- Any lines longer than {{config.maxLineLength}} characters will be truncated
- Results are returned using rg "" -n format, with line numbers starting at 1
{{#if (contains model.input_modalities "image")}}
- This tool allows Forge Code to read images (eg PNG, JPG, etc). When reading an image file the contents are presented visually.
- PDFs, Automatically encoded as base64 and sent as visual content for LLM to analyze pages. Any PDFs larger than {{config.maxImageSize}} bytes will return error
{{/if}}
- Jupyter notebooks (.ipynb files) are read as plain JSON text - you can parse the cell structure, outputs, and embedded content directly from the JSON
- This tool can only read files, not directories. To read a directory, use an ls command via the `{{tool_names.shell}}` tool.
- You can call multiple tools in a single response. It is always better to speculatively read multiple potentially useful files in parallel.
