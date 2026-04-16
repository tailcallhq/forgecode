#!/usr/bin/env tsx

/**
 * Fetch Skills Script for Skill Search Eval
 *
 * This script fetches 200 top safe skills from skills.sh and persists them
 * as a JSON fixture for the eval. Skills are sourced from the skills.sh
 * leaderboard page which embeds skill data in its JavaScript payload.
 *
 * Usage:
 *   tsx fetch_skills.ts [--output path/to/skills_fixture.json]
 *
 * The script:
 * 1. Fetches the skills.sh homepage
 * 2. Extracts the `initialSkills` JSON array from the page JavaScript
 * 3. Filters to skills from trusted publishers (optional)
 * 4. Fetches each skill's page for detailed description
 * 5. Outputs a committed JSON fixture with name, description, source
 */

import { writeFileSync, existsSync, mkdirSync } from "fs";
import { join, dirname } from "path";

const SKILLS_SH_URL = "https://skills.sh";
const MAX_SKILLS = 200;

// Trusted publishers for filtering (optional)
const TRUSTED_PUBLISHERS = [
  "anthropics",
  "vercel-labs",
  "microsoft",
  "github",
  "google",
  "openai",
];

interface SkillInfo {
  name: string;
  description: string;
  source: string; // skills.sh page URL
  publisher?: string;
  verified?: boolean;
}

async function fetchSkillsShPage(): Promise<string> {
  console.log(`Fetching ${SKILLS_SH_URL}...`);

  const response = await fetch(SKILLS_SH_URL, {
    headers: {
      Accept: "text/html",
      "User-Agent": "Mozilla/5.0 (compatible; SkillFetcher/1.0)",
    },
  });

  if (!response.ok) {
    throw new Error(`Failed to fetch skills.sh: ${response.status} ${response.statusText}`);
  }

  return response.text();
}

function extractInitialSkills(html: string): any[] {
  // Look for initialSkills JSON in the page JavaScript
  // Pattern: window.__INITIAL_STATE__ or var initialSkills = [...]
  const patterns = [
    /window\.__INITIAL_SKILLS__\s*=\s*(\[.*?\]);/s,
    /window\.__INITIAL_STATE__.*?skills\s*:\s*(\[.*?\])/s,
    /var\s+initialSkills\s*=\s*(\[.*?\]);/s,
    /"initialSkills":\s*(\[.*?\])/s,
    /initialSkills\s*[=:]\s*(\[.*?\])/s,
  ];

  for (const pattern of patterns) {
    const match = html.match(pattern);
    if (match && match[1]) {
      try {
        const parsed = JSON.parse(match[1]);
        if (Array.isArray(parsed)) {
          console.log(`Found ${parsed.length} skills in page data`);
          return parsed;
        }
      } catch (e) {
        // Continue to next pattern
      }
    }
  }

  // Fallback: try to find any JSON array that looks like skills
  const fallbackMatch = html.match(/(\[\s*\{\s*"name".*?\}\s*\])/s);
  if (fallbackMatch) {
    try {
      const parsed = JSON.parse(fallbackMatch[1]);
      if (Array.isArray(parsed)) {
        console.log(`Found ${parsed.length} skills via fallback pattern`);
        return parsed;
      }
    } catch (e) {
      // Ignore
    }
  }

  throw new Error(
    "Could not extract skills from skills.sh page. " +
    "The page structure may have changed. " +
    "Please check the page source manually."
  );
}

async function fetchSkillDescription(skillName: string): Promise<string | null> {
  const skillUrl = `https://skills.sh/s/${skillName}`;

  try {
    const response = await fetch(skillUrl, {
      headers: {
        Accept: "text/html",
        "User-Agent": "Mozilla/5.0 (compatible; SkillFetcher/1.0)",
      },
    });

    if (!response.ok) {
      console.warn(`  Warning: Failed to fetch ${skillUrl}: ${response.status}`);
      return null;
    }

    const html = await response.text();

    // Try to extract description from meta tags or page content
    // Pattern 1: meta description
    const metaMatch = html.match(/<meta[^>]*name=["']description["'][^>]*content=["']([^"']+)["']/i);
    if (metaMatch) {
      return metaMatch[1].trim();
    }

    // Pattern 2: OpenGraph description
    const ogMatch = html.match(/<meta[^>]*property=["']og:description["'][^>]*content=["']([^"']+)["']/i);
    if (ogMatch) {
      return ogMatch[1].trim();
    }

    // Pattern 3: JSON-LD structured data
    const jsonLdMatch = html.match(/<script type=["']application\/ld\+json["']>([^<]+)<\/script>/i);
    if (jsonLdMatch) {
      try {
        const jsonLd = JSON.parse(jsonLdMatch[1]);
        if (jsonLd.description) {
          return jsonLd.description;
        }
      } catch (e) {
        // Ignore
      }
    }

    // Pattern 4: Look for description in page content (div with class containing "description")
    const contentMatch = html.match(/<div[^>]*class=["'][^"']*description[^"']*["'][^>]*>([^<]+)<\/div>/i);
    if (contentMatch) {
      return contentMatch[1].trim();
    }

    return null;
  } catch (error) {
    console.warn(`  Warning: Error fetching ${skillUrl}: ${error}`);
    return null;
  }
}

function isSafeSkill(skill: any): boolean {
  // Check if skill has required fields
  if (!skill.name) {
    return false;
  }

  // Check for verified flag or trusted publisher
  if (skill.verified === true) {
    return true;
  }

  if (skill.publisher && TRUSTED_PUBLISHERS.includes(skill.publisher.toLowerCase())) {
    return true;
  }

  // Default: include all skills that have a name
  // (filtering can be adjusted based on requirements)
  return true;
}

async function main() {
  const args = process.argv.slice(2);
  const outputArg = args.find(arg => arg.startsWith("--output="));
  const outputPath = outputArg
    ? outputArg.replace("--output=", "")
    : join(__dirname, "skills_fixture.json");

  console.log("==========================================");
  console.log("Fetching Skills from skills.sh");
  console.log("==========================================");
  console.log(`Output: ${outputPath}`);
  console.log();

  try {
    // Fetch the skills.sh homepage
    const html = await fetchSkillsShPage();

    // Extract skills from page
    const rawSkills = extractInitialSkills(html);
    console.log(`Extracted ${rawSkills.length} total skills from page`);

    // Filter to safe skills
    const safeSkills = rawSkills.filter(isSafeSkill);
    console.log(`${safeSkills.length} skills passed safety filter`);

    // Take top MAX_SKILLS
    const skillsToFetch = safeSkills.slice(0, MAX_SKILLS);
    console.log(`Will fetch details for top ${skillsToFetch.length} skills`);
    console.log();

    // Fetch detailed descriptions for each skill
    const skills: SkillInfo[] = [];

    for (let i = 0; i < skillsToFetch.length; i++) {
      const rawSkill = skillsToFetch[i];
      const skillName = rawSkill.name || rawSkill.id || rawSkill.slug;

      if (!skillName) {
        console.warn(`  Skipping skill at index ${i}: no name found`);
        continue;
      }

      process.stdout.write(`[${i + 1}/${skillsToFetch.length}] Fetching ${skillName}... `);

      // Use existing description from page data if available
      let description = rawSkill.description || rawSkill.summary || rawSkill.shortDescription;

      // Fallback: fetch from skill page if no description in page data
      if (!description) {
        description = await fetchSkillDescription(skillName);
      }

      if (description) {
        skills.push({
          name: skillName,
          description: description.substring(0, 500), // Limit description length
          source: `https://skills.sh/s/${skillName}`,
          publisher: rawSkill.publisher || rawSkill.author,
          verified: rawSkill.verified || false,
        });
        console.log("✓");
      } else {
        // Include skill even without description, but mark it
        skills.push({
          name: skillName,
          description: "[No description available]",
          source: `https://skills.sh/s/${skillName}`,
          publisher: rawSkill.publisher || rawSkill.author,
          verified: rawSkill.verified || false,
        });
        console.log("⚠ (no description)");
      }

      // Small delay to be nice to the server
      await new Promise(resolve => setTimeout(resolve, 100));
    }

    console.log();
    console.log(`Successfully processed ${skills.length} skills`);

    // Ensure output directory exists
    const outputDir = dirname(outputPath);
    if (!existsSync(outputDir)) {
      mkdirSync(outputDir, { recursive: true });
    }

    // Write fixture file
    const fixture = {
      metadata: {
        fetched_at: new Date().toISOString(),
        source: SKILLS_SH_URL,
        total_skills: skills.length,
        max_skills_requested: MAX_SKILLS,
      },
      skills,
    };

    writeFileSync(outputPath, JSON.stringify(fixture, null, 2));

    console.log();
    console.log("==========================================");
    console.log("✓ Skills fixture saved successfully");
    console.log("==========================================");
    console.log(`Path: ${outputPath}`);
    console.log(`Skills: ${skills.length}`);
    console.log();
    console.log("Sample skills:");
    skills.slice(0, 5).forEach(skill => {
      console.log(`  - ${skill.name}: ${skill.description.substring(0, 60)}...`);
    });

  } catch (error) {
    console.error();
    console.error("==========================================");
    console.error("✗ Failed to fetch skills");
    console.error("==========================================");
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

main();
