use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use forge_app::domain::Skill;
use forge_app::{EnvironmentInfra, FileInfoInfra, FileReaderInfra, Walker, WalkerInfra};
use forge_domain::SkillRepository;
use futures::future::join_all;
use gray_matter::Matter;
use gray_matter::engine::YAML;
use serde::Deserialize;

/// Repository implementation for loading skills from multiple sources:
/// 1. Built-in skills (embedded in the application)
/// 2. Global custom skills (from ~/forge/skills/ directory)
/// 3. Agents skills (from ~/.agents/skills/ directory)
/// 4. Project-local skills (from .forge/skills/ directory in current working
///    directory)
///
/// ## Skill Precedence
/// When skills have duplicate names across different sources, the precedence
/// order is: **CWD (project-local) > Agents (~/.agents/skills) > Global
/// custom > Built-in**
///
/// This means project-local skills can override agents skills, which can
/// override global skills, which can override built-in skills.
///
/// ## Directory Resolution
/// - **Built-in skills**: Embedded in application binary
/// - **Global skills**: `~/forge/skills/<skill-name>/SKILL.md`
/// - **Agents skills**: `~/.agents/skills/<skill-name>/SKILL.md`
/// - **CWD skills**: `./.forge/skills/<skill-name>/SKILL.md` (relative to
///   current working directory)
///
/// Missing directories are handled gracefully and don't prevent loading from
/// other sources.
pub struct ForgeSkillRepository<I> {
    infra: Arc<I>,
}

impl<I> ForgeSkillRepository<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }

    /// Loads built-in skills that are embedded in the application
    fn load_builtin_skills(&self) -> Vec<Skill> {
        let builtin_skills = vec![
            (
                "forge://skills/create-skill/SKILL.md",
                include_str!("skills/create-skill/SKILL.md"),
            ),
            (
                "forge://skills/execute-plan/SKILL.md",
                include_str!("skills/execute-plan/SKILL.md"),
            ),
            (
                "forge://skills/github-pr-description/SKILL.md",
                include_str!("skills/github-pr-description/SKILL.md"),
            ),
            (
                "forge://skills/verification-specialist/SKILL.md",
                include_str!("skills/verification-specialist/SKILL.md"),
            ),
            (
                "forge://skills/constraint-enforcer/SKILL.md",
                include_str!("skills/constraint-enforcer/SKILL.md"),
            ),
            (
                "forge://skills/edge-case-tester/SKILL.md",
                include_str!("skills/edge-case-tester/SKILL.md"),
            ),
            (
                "forge://skills/reverse-engineering-helper/SKILL.md",
                include_str!("skills/reverse-engineering-helper/SKILL.md"),
            ),
            (
                "forge://skills/ml-model-debugging/SKILL.md",
                include_str!("skills/ml-model-debugging/SKILL.md"),
            ),
        ];

        builtin_skills
            .into_iter()
            .filter_map(|(path, content)| extract_skill(path, content))
            .collect()
    }
}

#[async_trait::async_trait]
impl<I: FileInfoInfra + EnvironmentInfra + FileReaderInfra + WalkerInfra> SkillRepository
    for ForgeSkillRepository<I>
{
    /// Loads all available skills from the skills directory
    ///
    /// # Errors
    /// Returns an error if skill loading fails
    async fn load_skills(&self) -> anyhow::Result<Vec<Skill>> {
        let mut skills = Vec::new();
        let env = self.infra.get_environment();

        // Load built-in skills
        let builtin_skills = self.load_builtin_skills();
        skills.extend(builtin_skills);

        // Load global skills
        let global_dir = env.global_skills_path();
        let global_skills = self.load_skills_from_dir(&global_dir).await?;
        skills.extend(global_skills);

        // Load agents skills (~/.agents/skills)
        if let Some(agents_dir) = env.agents_skills_path() {
            let agents_skills = self.load_skills_from_dir(&agents_dir).await?;
            skills.extend(agents_skills);
        }

        // Load project-local skills
        let cwd_dir = env.local_skills_path();
        let cwd_skills = self.load_skills_from_dir(&cwd_dir).await?;
        skills.extend(cwd_skills);

        // Resolve conflicts by keeping the last occurrence (CWD > Agents > Global >
        // Built-in)
        let skills = resolve_skill_conflicts(skills);

        // Render all skills with environment context
        let rendered_skills = skills
            .into_iter()
            .map(|skill| self.render_skill(skill, &env))
            .collect::<Vec<_>>();

        Ok(sort_skills(rendered_skills))
    }
}

impl<I: FileInfoInfra + EnvironmentInfra + FileReaderInfra + WalkerInfra> ForgeSkillRepository<I> {
    /// Loads skills from a specific directory by listing subdirectories first,
    /// then reading SKILL.md from each subdirectory if it exists
    async fn load_skills_from_dir(&self, dir: &std::path::Path) -> anyhow::Result<Vec<Skill>> {
        if !self.infra.exists(dir).await? {
            return Ok(vec![]);
        }

        let walker = Walker::unlimited()
            .cwd(dir.to_path_buf())
            .max_depth(1_usize)
            .max_breadth(usize::MAX); // Override breadth limit to see all skill directories
        let entries = self
            .infra
            .walk(walker)
            .await
            .with_context(|| format!("Failed to list directory: {}", dir.display()))?;

        // Filter for directories only (entries that end with '/')
        let mut subdirs: Vec<_> = entries
            .into_iter()
            .filter_map(|walked| {
                if walked.is_dir() && !walked.path.is_empty() {
                    // Construct the full path
                    Some(dir.join(&walked.path))
                } else {
                    None
                }
            })
            .collect();
        sort_paths(&mut subdirs);

        // Read SKILL.md from each subdirectory in parallel
        let futures = subdirs.into_iter().map(|subdir| {
            let infra = Arc::clone(&self.infra);
            async move {
                let skill_path = subdir.join("SKILL.md");

                // Check if SKILL.md exists in this subdirectory
                if infra.exists(&skill_path).await? {
                    // Read the file content
                    match infra.read_utf8(&skill_path).await {
                        Ok(content) => {
                            let path_str = skill_path.display().to_string();
                            let skill_name = subdir
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("unknown")
                                .to_string();

                            // Get all resource files in the skill directory recursively
                            let walker = Walker::unlimited().cwd(subdir.clone());
                            let mut resources = infra
                                .walk(walker)
                                .await
                                .unwrap_or_default()
                                .into_iter()
                                .filter_map(|walked| {
                                    // Only include files (not directories) and exclude SKILL.md
                                    if !walked.is_dir() {
                                        let full_path = subdir.join(&walked.path);
                                        if full_path.file_name() != skill_path.file_name() {
                                            Some(full_path)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>();
                            sort_paths(&mut resources);

                            // Try to extract skill from front matter, otherwise create with
                            // directory name
                            if let Some(skill) = extract_skill(&path_str, &content) {
                                Ok(Some(skill.resources(resources)))
                            } else {
                                // Fallback: create skill with directory name if front matter is
                                // missing
                                Ok(Some(
                                    Skill::new(skill_name, content, String::new())
                                        .path(path_str)
                                        .resources(resources),
                                ))
                            }
                        }
                        Err(e) => {
                            // Log warning but continue processing other skills
                            tracing::warn!(
                                "Failed to read skill file {}: {}",
                                skill_path.display(),
                                e
                            );
                            Ok(None)
                        }
                    }
                } else {
                    Ok(None)
                }
            }
        });

        // Execute all futures in parallel and collect results
        let results = join_all(futures).await;
        let skills: Vec<Skill> = results
            .into_iter()
            .filter_map(|result: anyhow::Result<Option<Skill>>| result.ok().flatten())
            .collect();

        Ok(skills)
    }

    /// Renders a skill's command field with environment context
    ///
    /// # Arguments
    /// * `skill` - The skill to render
    /// * `env` - The environment containing path informations
    fn render_skill(&self, skill: Skill, env: &forge_domain::Environment) -> Skill {
        let global = env.global_skills_path().display().to_string();
        let agents = env
            .agents_skills_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let local = env.local_skills_path().display().to_string();

        let rendered = skill
            .command
            .replace("{{global_skills_path}}", &global)
            .replace("{{agents_skills_path}}", &agents)
            .replace("{{local_skills_path}}", &local);

        skill.command(rendered)
    }
}

/// Private type for parsing skill YAML front matter
#[derive(Debug, Deserialize)]
struct SkillMetadata {
    /// Optional name of the skill (overrides filename if present)
    name: Option<String>,
    /// Optional description of the skill
    description: Option<String>,
}

/// Extracts metadata from the skill markdown content using YAML front matter
///
/// Parses YAML front matter from the markdown content and extracts skill
/// metadata. Expected format:
/// ```markdown
/// ---
/// name: "skill-name"
/// description: "Your description here"
/// ---
/// # Skill content...
/// ```
///
/// Returns a tuple of (name, description) where both are Option<String>.
fn extract_skill(path: &str, content: &str) -> Option<Skill> {
    let matter = Matter::<YAML>::new();
    let result = matter.parse::<SkillMetadata>(content);
    result.ok().and_then(|parsed| {
        let command = parsed.content;
        parsed
            .data
            .and_then(|data| data.name.zip(data.description))
            .map(|(name, description)| Skill::new(name, command, description).path(path))
    })
}

/// Resolves skill conflicts by keeping the last occurrence of each skill name
///
/// This gives precedence to later sources (CWD > Global)
fn resolve_skill_conflicts(skills: Vec<Skill>) -> Vec<Skill> {
    let mut seen = std::collections::HashMap::new();
    let mut result = Vec::new();

    for skill in skills {
        if let Some(idx) = seen.get(&skill.name) {
            // Replace the earlier skill with the same name
            if let Some(existing) = result.get_mut(*idx) {
                *existing = skill.clone();
            }
        } else {
            // First occurrence of this skill name
            seen.insert(skill.name.clone(), result.len());
            result.push(skill);
        }
    }

    result
}

fn sort_skills(mut skills: Vec<Skill>) -> Vec<Skill> {
    for skill in &mut skills {
        sort_paths(&mut skill.resources);
    }

    skills.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| path_sort_key(a.path.as_deref()).cmp(&path_sort_key(b.path.as_deref())))
            .then_with(|| a.description.cmp(&b.description))
    });

    skills
}

fn sort_paths(paths: &mut [PathBuf]) {
    paths.sort_by_key(|a| path_sort_key(Some(a.as_path())));
}

fn path_sort_key(path: Option<&Path>) -> String {
    path.map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use forge_config::ForgeConfig;
    use forge_infra::ForgeInfra;
    use pretty_assertions::assert_eq;

    use super::*;

    fn fixture_skill_repo() -> (ForgeSkillRepository<ForgeInfra>, std::path::PathBuf) {
        let skill_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/fixtures/skills_with_resources");
        let config = ForgeConfig::read().unwrap_or_default();
        let infra = Arc::new(ForgeInfra::new(std::env::current_dir().unwrap(), config));
        let repo = ForgeSkillRepository::new(infra);
        (repo, skill_dir)
    }

    #[test]
    fn test_resolve_skill_conflicts() {
        // Fixture
        let skills = vec![
            Skill::new("skill1", "global prompt", "global desc").path("/global/skill1.md"),
            Skill::new("skill2", "prompt2", "desc2").path("/global/skill2.md"),
            Skill::new("skill1", "cwd prompt", "cwd desc").path("/cwd/skill1.md"),
        ];

        // Act
        let actual = resolve_skill_conflicts(skills);

        // Assert
        assert_eq!(actual.len(), 2);
        assert_eq!(actual[0].name, "skill1");
        assert_eq!(
            actual[0].path,
            Some(std::path::Path::new("/cwd/skill1.md").to_path_buf())
        );
        assert_eq!(actual[0].command, "cwd prompt");
        assert_eq!(actual[1].name, "skill2");
    }

    #[test]
    fn test_sort_skills_orders_names_and_resources() {
        // Fixture
        let fixture = vec![
            Skill::new("zeta", "prompt", "desc")
                .path("/tmp/zeta/SKILL.md")
                .resources(vec![
                    PathBuf::from("/tmp/zeta/b.txt"),
                    PathBuf::from("/tmp/zeta/a.txt"),
                ]),
            Skill::new("alpha", "prompt", "desc").path("/tmp/alpha/SKILL.md"),
        ];

        // Act
        let actual = sort_skills(fixture);

        // Assert
        let expected = vec![
            Skill::new("alpha", "prompt", "desc").path("/tmp/alpha/SKILL.md"),
            Skill::new("zeta", "prompt", "desc")
                .path("/tmp/zeta/SKILL.md")
                .resources(vec![
                    PathBuf::from("/tmp/zeta/a.txt"),
                    PathBuf::from("/tmp/zeta/b.txt"),
                ]),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_load_builtin_skills() {
        // Fixture
        let repo = ForgeSkillRepository { infra: Arc::new(()) };

        // Act
        let actual = repo.load_builtin_skills();

        // Assert
        let expected = [
            ("create-skill", "forge://skills/create-skill/SKILL.md"),
            ("execute-plan", "forge://skills/execute-plan/SKILL.md"),
            (
                "github-pr-description",
                "forge://skills/github-pr-description/SKILL.md",
            ),
            (
                "verification-specialist",
                "forge://skills/verification-specialist/SKILL.md",
            ),
            (
                "constraint-enforcer",
                "forge://skills/constraint-enforcer/SKILL.md",
            ),
            (
                "edge-case-tester",
                "forge://skills/edge-case-tester/SKILL.md",
            ),
            (
                "reverse-engineering-helper",
                "forge://skills/reverse-engineering-helper/SKILL.md",
            ),
            (
                "ml-model-debugging",
                "forge://skills/ml-model-debugging/SKILL.md",
            ),
        ];

        assert_eq!(actual.len(), expected.len());

        for (name, path) in expected {
            let skill = actual
                .iter()
                .find(|skill| skill.name == name)
                .unwrap_or_else(|| panic!("missing built-in skill: {name}"));

            assert_eq!(skill.path, Some(std::path::Path::new(path).to_path_buf()));
            assert!(!skill.description.is_empty());
            assert!(!skill.command.is_empty());
        }
    }

    #[tokio::test]
    async fn test_extract_skill_with_valid_metadata() {
        // Fixture
        let path = "fixtures/skills/with_name_and_description.md";
        let content =
            forge_test_kit::fixture!("/src/fixtures/skills/with_name_and_description.md").await;

        // Act
        let actual = extract_skill(path, &content);

        // Assert
        let expected = Some(
            Skill::new(
                "pdf-handler",
                "# PDF Handler\n\nContent here...",
                "This is a skill for handling PDF files",
            )
            .path(path),
        );
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_extract_skill_with_incomplete_metadata() {
        // Fixture
        let content = forge_test_kit::fixture!("/src/fixtures/skills/with_name_only.md").await;

        // Act
        let actual = extract_skill("test.md", &content);

        // Assert - Returns None because metadata is incomplete
        assert_eq!(actual, None);
    }

    #[tokio::test]
    async fn test_load_skills_from_dir() {
        // Fixture
        let (repo, skill_dir) = fixture_skill_repo();

        // Act
        let actual = repo.load_skills_from_dir(&skill_dir).await.unwrap();

        // Assert - should load all skills
        assert_eq!(actual.len(), 2); // minimal-skill, test-skill
        assert_eq!(
            actual
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>(),
            vec!["minimal-skill", "test-skill"]
        );

        // Verify skill with no resources
        let minimal_skill = actual.iter().find(|s| s.name == "minimal-skill").unwrap();
        assert_eq!(minimal_skill.resources.len(), 0);

        // Verify skill with nested resources
        let test_skill = actual.iter().find(|s| s.name == "test-skill").unwrap();
        assert_eq!(test_skill.description, "A test skill with resources");
        assert_eq!(test_skill.resources.len(), 3); // file_1.txt, foo/file_2.txt, foo/bar/file_3.txt
        assert_eq!(
            test_skill
                .resources
                .iter()
                .map(|path| path
                    .strip_prefix(&skill_dir)
                    .unwrap()
                    .to_string_lossy()
                    .to_string())
                .collect::<Vec<_>>(),
            vec![
                "test-skill/file_1.txt".to_string(),
                "test-skill/foo/bar/file_3.txt".to_string(),
                "test-skill/foo/file_2.txt".to_string(),
            ]
        );

        // Ensure SKILL.md is never included in resources
        assert!(actual.iter().all(|s| {
            !s.resources
                .iter()
                .any(|p| p.file_name().unwrap() == "SKILL.md")
        }));
    }
}
