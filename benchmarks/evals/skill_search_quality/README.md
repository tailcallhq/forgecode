# Skill Search Quality Evaluation

This evaluation tests the quality of the `skill_search` tool by verifying that it correctly identifies and ranks relevant skills from the local skill library based on user task descriptions.

## Overview

The skill search eval uses **deterministic validation** (no LLM judge) to verify:

1. The `skill_search` tool is called when appropriate
2. Expected skills appear in the returned results
3. The most relevant skill (`must_be_first`) is ranked #1

## Files

| File | Purpose |
|------|---------|
| `task.yml` | Main eval definition with 20 test cases covering diverse intents (PDF generation, testing, deployment, web design, etc.) |
| `run_eval.sh` | Helper script to run eval validation on a context.json file |
| `test_validation.sh` | Unit tests for the validation logic (no eval run required) |
| `fetch_skills.ts` | **Offline script** to fetch 200 skills from skills.sh for ground truth generation |
| `generate_test_cases.ts` | **Offline script** to generate expected_skills using LLM (one-time setup) |

## Running the Eval

### Quick Validation Test

Test the validation logic without a live eval run:

```bash
./test_validation.sh
```

### Run Full Eval

The eval is typically run through the forge eval framework using `task.yml`. The validation uses pure shell/jq:

```bash
# Run via forgee with debug output
FORGE_DEBUG_REQUESTS='./context.json' forgee -p "I want to create a PDF document"

# Validate the results
./run_eval.sh ./context.json
```

## Test Case Structure

Each test case in `task.yml` has:

- `task`: Natural language description of what the user wants to achieve
- `expected_skills`: Comma-separated list of skill names that should appear in results
- `must_be_first`: The single skill that must be ranked #1

Example:
```yaml
- task: "I want to create a PDF document from markdown content"
  expected_skills: "pdf,docx,markdown,generate-document"
  must_be_first: "pdf"
```

## Validation Flow

1. **Tool Usage Check**: Verify `skill_search` was called using jq on `context.json`
2. **Expected Skills Check**: Extract skill names from tool response and verify all expected skills are present
3. **Ranking Check**: Verify `must_be_first` skill appears as the first result

## Regenerating Test Cases

If you need to update the expected skills (e.g., when skills are added/removed from the local library or when the prompt construction changes):

1. Fetch the latest skills from skills.sh:
   ```bash
   npx tsx fetch_skills.ts
   ```

2. Generate new test cases with LLM:
   ```bash
   npx tsx generate_test_cases.ts --skills=skills_fixture.json --output=test_cases.yml
   ```

3. Review the generated `test_cases.yml` and update `task.yml` with the new expected_skills

4. Commit the updated fixture and test cases

## Design Principles

- **No LLM at eval runtime**: All validation is deterministic shell/jq
- **Ground truth via LLM offline**: Expected skills generated once via LLM, committed as static data
- **Cross-platform**: Uses portable shell commands (sed, grep, cut, tr) compatible with both GNU and BSD tools
- **Reproducible**: Same inputs always produce same validation results

## Coverage

The 20 test cases cover:

- **Document Generation**: PDF, DOCX, resume creation
- **Testing**: Unit tests, TDD, E2E with Playwright
- **Deployment & DevOps**: CI/CD, Docker, Kubernetes, Terraform
- **Web Development**: Web design, landing pages, Next.js
- **Databases**: Schema design, migrations
- **Code Quality**: Code review, refactoring, static analysis
- **APIs**: REST API design, GraphQL
- **Mobile**: React Native
- **Authentication**: OAuth, JWT
- **Monitoring**: Logging, observability, alerting
- **Data Viz**: Charts and visualizations
- **CLI Tools**: Command-line utilities
- **Real-time**: WebSockets, chat applications
- **Performance**: Caching with Redis, SEO optimization
- **Automation**: Email automation systems
