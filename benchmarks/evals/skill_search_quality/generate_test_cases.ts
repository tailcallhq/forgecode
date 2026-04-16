#!/usr/bin/env tsx

/**
 * Generate Test Cases for Skill Search Eval
 *
 * This offline-only script takes the 200-skill fixture and a list of task
 * prompts as input, then calls Gemini via Vertex AI to determine which skills
 * from the fixture are most relevant to each task, in priority order.
 *
 * Usage:
 *   tsx generate_test_cases.ts \
 *     --skills=skills_fixture.json \
 *     --output=test_cases.yml \
 *     [--model=gemini-2.0-flash]
 *
 * The output is a YAML file of test case definitions — one per task — with:
 *   - task: The natural language task prompt
 *   - expected_skills: Ordered list of skill names that should appear in results
 *   - must_be_first: The single skill that must be ranked #1
 *
 * This script is run once and its output is reviewed by a human and committed.
 * Re-run it whenever the skill fixture changes or prompt construction changes.
 * Do NOT run it during the eval pipeline.
 */

import { readFileSync, writeFileSync } from "fs";
import { join } from "path";
import { vertex } from "@ai-sdk/google-vertex";
import { generateObject } from "ai";
import { z } from "zod";

// Default task prompts for skill search eval
const DEFAULT_TASKS = [
  "I want to create a PDF document from markdown content",
  "Help me write unit tests for my code using test-driven development",
  "Deploy my application to production with proper CI/CD",
  "Generate a beautiful website from my content",
  "I need to design a database schema for my app",
  "Review my code for bugs and security issues",
  "Automate browser testing with Playwright",
  "Create an API endpoint with proper documentation",
  "Optimize my website for search engines (SEO)",
  "Build a mobile app using React Native",
  "Set up authentication with OAuth and JWT",
  "Monitor my application with logging and alerting",
  "Create data visualizations and charts",
  "Build a CLI tool with command-line arguments",
  "Containerize my application with Docker",
  "Set up a Kubernetes cluster for my services",
  "Implement caching with Redis",
  "Build a real-time chat application with WebSockets",
  "Create an email automation system",
  "Set up infrastructure as code with Terraform",
];

// Schema for LLM output
const TestCaseSchema = z.object({
  task: z.string().describe("The original task prompt"),
  reasoning: z.string().describe("Explanation of why these skills were selected"),
  expected_skills: z
    .array(z.string())
    .min(1)
    .max(5)
    .describe("Ordered list of relevant skill names (most relevant first)"),
  must_be_first: z
    .string()
    .describe("The single skill name that should be ranked #1"),
});

const TestCasesSchema = z.object({
  test_cases: z.array(TestCaseSchema),
});

type TestCase = z.infer<typeof TestCaseSchema>;
type SkillInfo = {
  name: string;
  description: string;
  source: string;
  publisher?: string;
  verified?: boolean;
};

function loadSkills(fixturePath: string): SkillInfo[] {
  const content = readFileSync(fixturePath, "utf-8");
  const fixture = JSON.parse(content);
  return fixture.skills;
}

async function generateTestCase(
  task: string,
  skills: SkillInfo[],
  model: string
): Promise<TestCase> {
  // Create a condensed view of skills for the LLM
  const skillsDescription = skills
    .map((s, i) => `${i + 1}. ${s.name}: ${s.description.substring(0, 150)}`)
    .join("\n");

  const prompt = `You are evaluating skill relevance for a coding assistant.

Task: "${task}"

Available Skills (${skills.length} total):
${skillsDescription}

Select the 3-5 MOST RELEVANT skills for this task, ordered by relevance (most relevant first).

Requirements:
1. The FIRST skill in your list (must_be_first) should be the single best match
2. Only include skills that directly help accomplish the task
3. Order matters - skills should be ranked by relevance
4. Be specific - prefer skills that directly match the task over general ones

Output your reasoning and the ordered list of skill names.`;

  try {
    const result = await generateObject({
      model: vertex(model, {}),
      schema: TestCaseSchema,
      prompt,
      temperature: 0.2, // Low temperature for consistency
    });

    return {
      task,
      reasoning: result.object.reasoning,
      expected_skills: result.object.expected_skills,
      must_be_first: result.object.must_be_first,
    };
  } catch (error) {
    console.error(`  Error generating test case for "${task}":`, error);
    throw error;
  }
}

async function main() {
  const args = process.argv.slice(2);

  const skillsArg = args.find((arg) => arg.startsWith("--skills="));
  const skillsPath = skillsArg
    ? skillsArg.replace("--skills=", "")
    : join(__dirname, "skills_fixture.json");

  const outputArg = args.find((arg) => arg.startsWith("--output="));
  const outputPath = outputArg
    ? outputArg.replace("--output=", "")
    : join(__dirname, "test_cases.yml");

  const modelArg = args.find((arg) => arg.startsWith("--model="));
  const model = modelArg ? modelArg.replace("--model=", "") : "gemini-2.0-flash";

  const tasksArg = args.find((arg) => arg.startsWith("--tasks="));
  const tasks = tasksArg
    ? readFileSync(tasksArg.replace("--tasks=", ""), "utf-8")
        .split("\n")
        .filter((t) => t.trim())
    : DEFAULT_TASKS;

  console.log("==========================================");
  console.log("Generate Test Cases for Skill Search Eval");
  console.log("==========================================");
  console.log(`Skills: ${skillsPath}`);
  console.log(`Output: ${outputPath}`);
  console.log(`Model: ${model}`);
  console.log(`Tasks: ${tasks.length}`);
  console.log();

  // Load skills
  console.log("Loading skills fixture...");
  const skills = loadSkills(skillsPath);
  console.log(`Loaded ${skills.length} skills`);
  console.log();

  // Generate test cases
  console.log("Generating test cases with LLM...");
  console.log("(This may take a few minutes)");
  console.log();

  const testCases: TestCase[] = [];

  for (let i = 0; i < tasks.length; i++) {
    const task = tasks[i];
    console.log(`[${i + 1}/${tasks.length}] Processing: ${task.substring(0, 50)}...`);

    try {
      const testCase = await generateTestCase(task, skills, model);
      testCases.push(testCase);
      console.log(`  ✓ Expected skills: ${testCase.expected_skills.join(", ")}`);
      console.log(`  ✓ Must be first: ${testCase.must_be_first}`);
    } catch (error) {
      console.error(`  ✗ Failed to generate test case`);
      // Continue with other tasks
    }

    console.log();
  }

  // Generate YAML output
  console.log("Generating YAML output...");

  let yaml = `# Skill Search Quality Eval - Test Cases\n`;
  yaml += `# Generated: ${new Date().toISOString()}\n`;
  yaml += `# Model: ${model}\n`;
  yaml += `# Skills Source: ${skillsPath}\n`;
  yaml += `# Total Test Cases: ${testCases.length}\n`;
  yaml += `\n`;
  yaml += `test_cases:\n`;

  for (const tc of testCases) {
    yaml += `  - task: ${JSON.stringify(tc.task)}\n`;
    yaml += `    reasoning: ${JSON.stringify(tc.reasoning)}\n`;
    yaml += `    expected_skills:\n`;
    for (const skill of tc.expected_skills) {
      yaml += `      - ${skill}\n`;
    }
    yaml += `    must_be_first: ${tc.must_be_first}\n`;
  }

  // Write output
  writeFileSync(outputPath, yaml);

  console.log();
  console.log("==========================================");
  console.log("✓ Test cases generated successfully");
  console.log("==========================================");
  console.log(`Path: ${outputPath}`);
  console.log(`Test cases: ${testCases.length}`);
  console.log();
  console.log("Review the generated test cases and commit them to version control.");
  console.log("These will be used as ground truth for deterministic eval validation.");
}

main().catch((error) => {
  console.error();
  console.error("==========================================");
  console.error("✗ Failed to generate test cases");
  console.error("==========================================");
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
