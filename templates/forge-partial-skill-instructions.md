## Skill Instructions:

**CRITICAL**: Before attempting any task, ALWAYS check if a skill exists for it in the available_skills list below. Skills are specialized workflows that must be invoked when their trigger conditions match the user's request.

How skills work: Use the `skill` tool with just the skill name parameter (e.g., `{"name": "constraint-enforcer"}`). The tool returns skill instructions — read and follow them.

Important:

- Only invoke skills listed in `<available_skills>` below
- Do not invoke a skill that is already active/loaded

### Mandatory Trigger

Before claiming a task is complete, invoke `verification-specialist` and run a real verification command.

<available_skills>
{{#each skills}}
<skill>
<name>{{this.name}}</name>
<description>
{{this.description}}
</description>
</skill>
{{/each}}
</available_skills>
