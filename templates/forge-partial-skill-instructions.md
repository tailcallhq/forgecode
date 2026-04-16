## Skill Instructions:

**CRITICAL**: Before attempting any task, ALWAYS check if a skill exists for it in the available_skills list below. Skills are specialized workflows that must be invoked when their trigger conditions match the user's request.

How skills work:

1. **Invocation**: Use the `skill` tool with just the skill name parameter

   - Example: Call skill tool with `{"name": "mock-calculator"}`
   - No additional arguments needed

2. **Response**: The tool returns the skill's details wrapped in `<skill_details>` containing:

   - `<command path="..."><![CDATA[...]]></command>` - The complete SKILL.md file content with the skill's path
   - `<resource>` tags - List of additional resource files available in the skill directory
   - Includes usage guidelines, instructions, and any domain-specific knowledge

3. **Action**: Read and follow the instructions provided in the skill content
   - The skill instructions will tell you exactly what to do and how to use the resources
   - Some skills provide workflows, others provide reference information
   - Apply the skill's guidance to complete the user's task

Examples of skill invocation:

- To invoke calculator skill: use skill tool with name "calculator"
- To invoke weather skill: use skill tool with name "weather"
- For namespaced skills: use skill tool with name "office-suite:pdf"

Important:

- Only invoke skills listed in `<available_skills>` below
- Do not invoke a skill that is already active/loaded
- Skills are not CLI commands - use the skill tool to load them
- After loading a skill, follow its specific instructions to help the user

