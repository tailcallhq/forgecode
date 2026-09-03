Use the following summary frames as the authoritative reference for all coding suggestions and decisions. Do not re-explain or revisit it unless I ask. Additional summary frames will be added as the conversation progress.

{{#if has_file_changes}}
## Files Modified

{{#each file_changes}}
{{#if additions}}
**`{{path}}`** (+{{additions}}, -{{deletions}})
{{else}}
**`{{path}}`** (modified)
{{/if}}
{{/each}}

{{/if}}

{{#if has_tool_results}}
## Operations

{{#each tool_results}}
{{#if is_error}}
⚠️ **{{tool_name}}** `{{path}}` - Failed: `{{error_summary}}`
{{else if is_shell}}
▶️ **Execute:** `{{command}}`
{{else if is_mcp}}
🔌 **MCP:** `{{mcp_name}}`
{{else if is_skill}}
🎯 **Skill:** `{{skill_name}}`
{{else}}
📝 **{{tool_name}}:** `{{path}}`
{{/if}}
{{/each}}

{{/if}}

{{#if has_todo_changes}}
## Task Progress

{{#each todo_summary}}
{{this}}
{{/each}}

{{/if}}

{{#if has_decisions}}
## Key Decisions

{{#each decisions}}
- {{this}}
{{/each}}

{{/if}}

{{#if has_context_continuity}}
## Context Continuity

- **Previous session:** {{previous_session_summary}}
- **Preserved state:** {{preserved_state}}
{{/if}}

---

## Prior Context Summary

{{#each messages}}
### {{inc @index}}. {{role}}

{{#each contents}}
{{#if text}}
```
{{text}}
```
{{/if}}
{{~#if tool_call}}
{{#if tool_call.tool.file_update}}
**Update:** `{{tool_call.tool.file_update.path}}`
{{else if tool_call.tool.file_read}}
**Read:** `{{tool_call.tool.file_read.path}}`
{{else if tool_call.tool.file_remove}}
**Delete:** `{{tool_call.tool.file_remove.path}}`
{{else if tool_call.tool.search}}
**Search:** `{{tool_call.tool.search.pattern}}`
{{else if tool_call.tool.skill}}
**Skill:** `{{tool_call.tool.skill.name}}`
{{else if tool_call.tool.sem_search}}
**Semantic Search:**
{{#each tool_call.tool.sem_search.queries}}
- `{{use_case}}`
{{/each}}
{{else if tool_call.tool.shell}}
**Execute:**
```
{{tool_call.tool.shell.command}}
```
{{else if tool_call.tool.mcp}}
**MCP:** `{{tool_call.tool.mcp.name}}`
{{else if tool_call.tool.todo_write}}
**Task Plan:**
{{#each tool_call.tool.todo_write.changes}}
{{#if (eq kind "added")}}
- [ADD] {{todo.content}}
{{else if (eq kind "updated")}}
{{#if (eq todo.status "completed")}}
- [DONE] ~~{{todo.content}}~~
{{else if (eq todo.status "in_progress")}}
- [IN_PROGRESS] {{todo.content}}
{{else}}
- [UPDATE] {{todo.content}}
{{/if}}
{{else if (eq kind "removed")}}
- [CANCELLED] ~~{{todo.content}}~~
{{/if}}
{{/each}}
{{/if~}}
{{/if~}}

{{/each}}

{{/each}}

---

Proceed with implementation based on this context.
