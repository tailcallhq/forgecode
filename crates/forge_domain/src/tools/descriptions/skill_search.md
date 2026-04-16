Searches for relevant skills from your available skill library based on what you want to achieve. Use this tool when you need to find a specialized skill for a task type but don't know the specific skill name. This tool sends your query along with all available skills (built-in, global, and project-local) to the forge backend, which returns the most relevant skills ranked by relevance.

**When to use:**
- Starting a new task type where you don't have a known skill
- Exploring what specialized workflows are available for your goal
- Looking for domain-specific best practices (PDF generation, testing, deployment, etc.)

**How it works:**
1. Call `skill_search` with a description like "generate a PDF report" or "write tests using TDD"
2. The tool queries the forge backend with all your available skills and the search query
3. Review the returned skills (name, description, relevance score)
4. Use the `skill` tool with the most relevant skill name to load its full instructions
5. Follow the skill instructions to complete your task

**Examples:**
- Task: "I want to create a PDF document" → `skill_search` with query "generate PDF from content"
- Task: "Help me write tests for this code" → `skill_search` with query "write tests using test-driven development"
- Task: "Deploy my app to production" → `skill_search` with query "deploy application"

After calling `skill_search`, always use the `skill` tool with the specific skill name to load and execute the skill's instructions.
