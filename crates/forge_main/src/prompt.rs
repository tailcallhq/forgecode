use std::borrow::Cow;
use std::fmt::Write;
use std::path::PathBuf;

use convert_case::{Case, Casing};
use derive_setters::Setters;
use forge_api::{AgentId, ModelId, Usage};
use forge_tracker::VERSION;
use nu_ansi_term::{Color, Style};
use reedline::{Prompt, PromptHistorySearchStatus};

use crate::display_constants::markers;

// Constants
const MULTILINE_INDICATOR: &str = "::: ";
const RIGHT_CHEVRON: &str = "❯";

/// Very Specialized Prompt for the Agent Chat
#[derive(Clone, Setters)]
#[setters(strip_option, borrow_self)]
pub struct ForgePrompt {
    pub cwd: PathBuf,
    pub usage: Option<Usage>,
    pub agent_id: AgentId,
    pub model: Option<ModelId>,
    pub git_branch: Option<String>,
}

impl ForgePrompt {
    /// Creates a new `ForgePrompt`, resolving the git branch once at
    /// construction time.
    pub fn new(cwd: PathBuf, agent_id: AgentId) -> Self {
        let git_branch = get_git_branch();
        Self { cwd, usage: None, agent_id, model: None, git_branch }
    }

    pub fn refresh(&mut self) -> &mut Self {
        let git_branch = get_git_branch();
        self.git_branch = git_branch;
        self
    }
}

impl Prompt for ForgePrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        // Pre-compute styles to avoid repeated style creation
        let mode_style = Style::new().fg(Color::White).bold();
        let folder_style = Style::new().fg(Color::Cyan);
        let branch_style = Style::new().fg(Color::LightGreen);

        // Get current directory
        let current_dir = self
            .cwd
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
            .unwrap_or_else(|| markers::EMPTY.to_string());

        // Use a string buffer to reduce allocations
        let mut result = String::with_capacity(64); // Pre-allocate a reasonable size

        // Build the string step-by-step
        write!(
            result,
            "{} {}",
            mode_style.paint(self.agent_id.as_str().to_case(Case::UpperSnake)),
            folder_style.paint(&current_dir)
        )
        .unwrap();

        // Only append branch info if present
        if let Some(branch) = self.git_branch.as_deref()
            && branch != current_dir
        {
            write!(result, " {} ", branch_style.paint(branch)).unwrap();
        }

        write!(result, "\n{} ", branch_style.paint(RIGHT_CHEVRON)).unwrap();

        Cow::Owned(result)
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        // Use a string buffer with pre-allocation to reduce allocations
        let mut result = String::with_capacity(32);

        // Start with bracket and version
        write!(result, "[{VERSION}").unwrap();

        // Append model if available
        if let Some(model) = self.model.as_ref() {
            let model_str = model.to_string();
            let formatted_model = model_str
                .split('/')
                .next_back()
                .unwrap_or_else(|| model.as_str());
            write!(result, "/{formatted_model}").unwrap();
        }

        if let Some(usage) = self.usage.as_ref().map(|usage| &usage.total_tokens) {
            write!(result, "/{usage}").unwrap();
        }

        write!(result, "]").unwrap();

        // Apply styling once at the end
        Cow::Owned(
            Style::new()
                .bold()
                .fg(Color::DarkGray)
                .paint(&result)
                .to_string(),
        )
    }

    fn render_prompt_indicator(&self, _prompt_mode: reedline::PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed(MULTILINE_INDICATOR)
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: reedline::PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };

        let mut result = String::with_capacity(32);

        // Handle empty search term more elegantly
        if history_search.term.is_empty() {
            write!(result, "({prefix}reverse-search) ").unwrap();
        } else {
            write!(
                result,
                "({}reverse-search: {}) ",
                prefix, history_search.term
            )
            .unwrap();
        }

        Cow::Owned(Style::new().fg(Color::White).paint(&result).to_string())
    }
}

/// Gets the current git branch name if available
fn get_git_branch() -> Option<String> {
    let repo = gix::discover(".").ok()?;
    let head = repo.head().ok()?;
    head.referent_name().map(|r| r.shorten().to_string())
}

#[cfg(test)]
mod tests {
    use std::env;

    use nu_ansi_term::Style;
    use pretty_assertions::assert_eq;

    use super::*;

    impl Default for ForgePrompt {
        fn default() -> Self {
            ForgePrompt {
                cwd: PathBuf::from("."),
                usage: None,
                agent_id: AgentId::default(),
                model: None,
                git_branch: None,
            }
        }
    }

    #[test]
    fn test_render_prompt_left() {
        let prompt = ForgePrompt::default();

        let actual = prompt.render_prompt_left();

        // Check that it has the expected format with mode and directory displayed
        assert!(actual.contains("FORGE"));
        assert!(actual.contains(RIGHT_CHEVRON));
    }

    #[test]
    fn test_render_prompt_left_with_custom_prompt() {
        // Set $PROMPT environment variable temporarily for this test
        unsafe {
            env::set_var("PROMPT", "CUSTOM_TEST_PROMPT");
        }

        let prompt = ForgePrompt::default();
        let actual = prompt.render_prompt_left();

        // Clean up after test
        unsafe {
            env::remove_var("PROMPT");
        }

        // Verify the prompt contains expected elements regardless of $PROMPT var
        assert!(actual.contains("FORGE"));
        assert!(actual.contains(RIGHT_CHEVRON));
    }

    #[test]
    fn test_render_prompt_right_with_usage() {
        let usage = Usage {
            prompt_tokens: forge_api::TokenCount::Actual(10),
            completion_tokens: forge_api::TokenCount::Actual(20),
            total_tokens: forge_api::TokenCount::Approx(30),
            ..Default::default()
        };
        let mut prompt = ForgePrompt::default();
        let _ = prompt.usage(usage);

        let actual = prompt.render_prompt_right();
        assert!(actual.contains(&VERSION.to_string()));
        assert!(actual.contains("~30"));
    }

    #[test]
    fn test_render_prompt_right_without_usage() {
        let prompt = ForgePrompt::default();
        let actual = prompt.render_prompt_right();
        assert!(actual.contains(&VERSION.to_string()));
        assert!(actual.contains("0"));
    }

    #[test]
    fn test_render_prompt_multiline_indicator() {
        let prompt = ForgePrompt::default();
        let actual = prompt.render_prompt_multiline_indicator();
        let expected = MULTILINE_INDICATOR;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_render_prompt_history_search_indicator_passing() {
        let prompt = ForgePrompt::default();
        let history_search = reedline::PromptHistorySearch {
            status: PromptHistorySearchStatus::Passing,
            term: "test".to_string(),
        };
        let actual = prompt.render_prompt_history_search_indicator(history_search);
        let expected = Style::new()
            .fg(Color::White)
            .paint("(reverse-search: test) ")
            .to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_render_prompt_history_search_indicator_failing() {
        let prompt = ForgePrompt::default();
        let history_search = reedline::PromptHistorySearch {
            status: PromptHistorySearchStatus::Failing,
            term: "test".to_string(),
        };
        let actual = prompt.render_prompt_history_search_indicator(history_search);
        let expected = Style::new()
            .fg(Color::White)
            .paint("(failing reverse-search: test) ")
            .to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_render_prompt_history_search_indicator_empty_term() {
        let prompt = ForgePrompt::default();
        let history_search = reedline::PromptHistorySearch {
            status: PromptHistorySearchStatus::Passing,
            term: "".to_string(),
        };
        let actual = prompt.render_prompt_history_search_indicator(history_search);
        let expected = Style::new()
            .fg(Color::White)
            .paint("(reverse-search) ")
            .to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_render_prompt_right_with_model() {
        let usage = Usage {
            prompt_tokens: forge_api::TokenCount::Actual(10),
            completion_tokens: forge_api::TokenCount::Actual(20),
            total_tokens: forge_api::TokenCount::Actual(30),
            ..Default::default()
        };
        let mut prompt = ForgePrompt::default();
        let _ = prompt.usage(usage);
        let _ = prompt.model(ModelId::new("anthropic/claude-3"));

        let actual = prompt.render_prompt_right();
        assert!(actual.contains("claude-3")); // Only the last part after splitting by '/'
        assert!(!actual.contains("anthropic/claude-3")); // Should not contain the full model ID
        assert!(actual.contains(&VERSION.to_string()));
        assert!(actual.contains("30"));
    }
}
