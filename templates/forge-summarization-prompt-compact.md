# Compact Context Summary (Low-Token Version)

Summarize the following conversation in 150 tokens or less.

Format:
- **Goal**: [What user wanted]
- **Decisions**: [Key choices made]
- **Files**: [Modified files with +/- prefix for create/delete]
- **Commands**: [Run commands]
- **Progress**: [What done/remaining]
- **Current**: [Current focus]

Context:
{{#each messages}}
[{{role}}]: {{text}}
{{/each}}

Summary:
