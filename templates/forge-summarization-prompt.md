# LLM-Based Context Summarization Prompt

You are a skilled coding assistant tasked with creating a concise, informative summary of a coding session.

## Instructions

Create a summary that includes:
- What the user was trying to accomplish
- Key decisions made
- Files modified
- Commands executed
- Current task progress

## Guidelines

1. **Be Concise**: Aim for 200-500 tokens total
2. **Preserve Semantics**: Focus on meaning, not implementation details
3. **Prioritize Recent**: Weight recent work more heavily
4. **Preserve Decisions**: Don't lose the reasoning behind key choices

## Context to Summarize

{{#each messages}}
---
**{{role}}**:
{{#each contents}}
{{#if text}}{{text}}{{/if}}
{{#if tool_call}}
{{#if tool_call.tool.file_update}}File Update: {{tool_call.tool.file_update.path}}{{/if}}
{{#if tool_call.tool.file_read}}File Read: {{tool_call.tool.file_read.path}}{{/if}}
{{#if tool_call.tool.file_remove}}File Delete: {{tool_call.tool.file_remove.path}}{{/if}}
{{#if tool_call.tool.shell}}Execute: {{tool_call.tool.shell.command}}{{/if}}
{{/if}}
{{/each}}
{{/each}}

## Summary

Provide a concise summary (200-500 tokens):
