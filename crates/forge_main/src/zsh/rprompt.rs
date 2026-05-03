//! ZSH right prompt implementation.
//!
//! Provides the right prompt (RPROMPT) display for the ZSH shell integration,
//! showing agent name, model, token count and reasoning effort information.
//!
//! The reasoning effort label is rendered in one of two forms depending on
//! the available terminal width: a three-letter abbreviation (e.g. `MED`,
//! `HIG`) on narrow terminals and the full uppercase label (e.g. `MEDIUM`,
//! `HIGH`) on wider terminals. See [`WIDE_TERMINAL_THRESHOLD`].

use std::fmt::{self, Display};

use convert_case::{Case, Casing};
use derive_setters::Setters;
use forge_config::ForgeConfig;
use forge_domain::{AgentId, Effort, ModelId, TokenCount};

use super::style::{ZshColor, ZshStyle};
use crate::utils::humanize_number;

/// ZSH right prompt displaying agent, model, token count and reasoning effort.
///
/// Formats shell prompt information with appropriate colors:
/// - Inactive state (no tokens): dimmed colors
/// - Active state (has tokens): bright white/cyan/yellow colors
///
/// The reasoning effort label adapts to the available terminal width: on
/// narrow terminals (< [`WIDE_TERMINAL_THRESHOLD`] columns) it is rendered
/// as a three-letter abbreviation, otherwise the full uppercase label is
/// shown. When [`ZshRPrompt::terminal_width`] is unset the full-length form
/// is used as a safe default.
#[derive(Setters)]
pub struct ZshRPrompt {
    agent: Option<AgentId>,
    model: Option<ModelId>,
    token_count: Option<TokenCount>,
    cost: Option<f64>,
    /// Currently configured reasoning effort level for the active model.
    /// Rendered to the right of the model when set.
    reasoning_effort: Option<Effort>,
    /// Terminal width in columns, used to pick between the compact
    /// three-letter label and the full-length uppercase label for
    /// reasoning effort. When `None`, the prompt falls back to the
    /// full-length form.
    terminal_width: Option<usize>,
    /// Controls whether to render nerd font symbols. Defaults to `true`.
    #[setters(into)]
    use_nerd_font: bool,
    /// Currency symbol for cost display (e.g., "INR", "EUR", "$", "€").
    /// Defaults to "$".
    #[setters(into)]
    currency_symbol: String,
    /// Conversion ratio for cost display. Cost is multiplied by this value.
    /// Defaults to 1.0.
    conversion_ratio: f64,
}
impl ZshRPrompt {
    /// Constructs a [`ZshRPrompt`] with currency settings populated from the
    /// provided [`ForgeConfig`].
    pub fn from_config(config: &ForgeConfig) -> Self {
        Self::default()
            .currency_symbol(config.currency_symbol.clone())
            .conversion_ratio(config.currency_conversion_rate.value())
    }
}

impl Default for ZshRPrompt {
    fn default() -> Self {
        Self {
            agent: None,
            model: None,
            token_count: None,
            cost: None,
            reasoning_effort: None,
            terminal_width: None,
            use_nerd_font: true,
            currency_symbol: "\u{f155}".to_string(),
            conversion_ratio: 1.0,
        }
    }
}

const AGENT_SYMBOL: &str = "\u{f167a}";
const MODEL_SYMBOL: &str = "\u{ec19}";

/// Terminal width (in columns) at which the reasoning effort label switches
/// from the compact three-letter form to the full uppercase label.
///
/// Widths greater than or equal to this threshold render the full label
/// (e.g. `MEDIUM`, `HIGH`); widths below it collapse to the first three
/// characters (e.g. `MED`, `HIG`). The value is intentionally a coarse
/// static threshold — typical RPROMPT content is around 40-50 visible
/// cells, so 100 columns leaves enough room on the left for most LPROMPTs
/// and comfortable typing space once the full label is shown.
const WIDE_TERMINAL_THRESHOLD: usize = 100;

impl Display for ZshRPrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let active = *self.token_count.unwrap_or_default() > 0usize;

        // Add agent
        let agent_id = self.agent.clone().unwrap_or_default();
        let agent_id = if self.use_nerd_font {
            format!(
                "{AGENT_SYMBOL} {}",
                agent_id.to_string().to_case(Case::UpperSnake)
            )
        } else {
            agent_id.to_string().to_case(Case::UpperSnake)
        };
        let styled = if active {
            agent_id.zsh().bold().fg(ZshColor::WHITE)
        } else {
            agent_id.zsh().bold().fg(ZshColor::DIMMED)
        };
        write!(f, " {}", styled)?;

        // Add token count
        if let Some(count) = self.token_count {
            let num = humanize_number(*count);

            let prefix = match count {
                TokenCount::Actual(_) => "",
                TokenCount::Approx(_) => "~",
            };

            if active {
                write!(f, " {}{}", prefix, num.zsh().fg(ZshColor::WHITE).bold())?;
            }
        }

        // Add cost
        if let Some(cost) = self.cost
            && active
        {
            let converted_cost = cost * self.conversion_ratio;
            let cost_str = format!("{}{:.2}", self.currency_symbol, converted_cost);
            write!(f, " {}", cost_str.zsh().fg(ZshColor::GREEN).bold())?;
        }

        // Add model
        if let Some(ref model_id) = self.model {
            let model_id = if self.use_nerd_font {
                format!("{MODEL_SYMBOL} {}", model_id)
            } else {
                model_id.to_string()
            };
            let styled = if active {
                model_id.zsh().fg(ZshColor::CYAN)
            } else {
                model_id.zsh().fg(ZshColor::DIMMED)
            };
            write!(f, " {}", styled)?;
        }

        // Add reasoning effort (rendered to the right of the model).
        // `Effort::None` is suppressed because it carries no useful information
        // for the user to see in the prompt. Below `WIDE_TERMINAL_THRESHOLD`
        // columns the label collapses to its first three characters so the
        // prompt stays compact on narrow terminals; above the threshold the
        // full uppercase label is rendered for readability.
        if let Some(ref effort) = self.reasoning_effort
            && !matches!(effort, Effort::None)
        {
            let is_wide =
                self.terminal_width.unwrap_or(WIDE_TERMINAL_THRESHOLD) >= WIDE_TERMINAL_THRESHOLD;
            // Use `chars().take(3).collect()` rather than `&label[..3]` to
            // satisfy the `clippy::string_slice` lint that is denied in CI.
            // `Effort` serializes as lowercase ASCII, so taking the first
            // three chars is always well-defined.
            let effort_label = if is_wide {
                effort.to_string().to_uppercase()
            } else {
                effort
                    .to_string()
                    .chars()
                    .take(3)
                    .collect::<String>()
                    .to_uppercase()
            };
            let styled = if active {
                effort_label.zsh().fg(ZshColor::YELLOW)
            } else {
                effort_label.zsh().fg(ZshColor::DIMMED)
            };
            write!(f, " {}", styled)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_rprompt_init_state() {
        // No tokens = init/dimmed state
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .to_string();

        let expected = " %B%F{240}\u{f167a} FORGE%f%b %F{240}\u{ec19} gpt-4%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_tokens() {
        // Tokens > 0 = active/bright state
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .to_string();

        let expected = " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %F{134}\u{ec19} gpt-4%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_tokens_and_cost() {
        // Tokens > 0 with cost = active/bright state with cost display
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .cost(Some(0.0123))
            .currency_symbol("\u{f155}")
            .to_string();

        let expected = " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %B%F{2}\u{f155}0.01%f%b %F{134}\u{ec19} gpt-4%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_without_nerdfonts() {
        // Test with nerdfonts disabled
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .use_nerd_font(false)
            .to_string();

        let expected = " %B%F{15}FORGE%f%b %B%F{15}1.5k%f%b %F{134}gpt-4%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_currency_conversion() {
        // Test with custom currency symbol and conversion ratio
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .cost(Some(0.01))
            .currency_symbol("INR")
            .conversion_ratio(83.5)
            .to_string();

        let expected = " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %B%F{2}INR0.83%f%b %F{134}\u{ec19} gpt-4%f";
        assert_eq!(actual, expected);
    }
    #[test]
    fn test_rprompt_with_eur_currency() {
        // Test with EUR currency
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .cost(Some(0.01))
            .currency_symbol("€")
            .conversion_ratio(0.92)
            .to_string();

        let expected = " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %B%F{2}€0.01%f%b %F{134}\u{ec19} gpt-4%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_reasoning_effort_active() {
        // Active state (tokens > 0) renders reasoning effort in YELLOW to the
        // right of the model.
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .reasoning_effort(Some(Effort::High))
            .to_string();

        let expected =
            " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %F{134}\u{ec19} gpt-4%f %F{3}HIGH%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_reasoning_effort_init_state() {
        // Inactive state (no tokens) renders reasoning effort DIMMED.
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .reasoning_effort(Some(Effort::Medium))
            .to_string();

        let expected = " %B%F{240}\u{f167a} FORGE%f%b %F{240}\u{ec19} gpt-4%f %F{240}MEDIUM%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_reasoning_effort_without_nerdfonts() {
        // Nerd fonts disabled: agent and model lose their glyph prefixes;
        // the reasoning effort remains as a plain uppercase color-coded label.
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .reasoning_effort(Some(Effort::Low))
            .use_nerd_font(false)
            .to_string();

        let expected = " %B%F{15}FORGE%f%b %B%F{15}1.5k%f%b %F{134}gpt-4%f %F{3}LOW%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_reasoning_effort_none_variant_is_hidden() {
        // `Effort::None` is semantically "no reasoning" and carries no display
        // value, so the rprompt suppresses it entirely.
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .reasoning_effort(Some(Effort::None))
            .to_string();

        let expected = " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %F{134}\u{ec19} gpt-4%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_without_reasoning_effort_is_hidden() {
        // When no reasoning effort is set, nothing is appended after the model.
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .reasoning_effort(None)
            .to_string();

        let expected = " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %F{134}\u{ec19} gpt-4%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_reasoning_effort_xhigh() {
        // `Effort::XHigh` renders as the uppercase string "XHIGH".
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .reasoning_effort(Some(Effort::XHigh))
            .to_string();

        let expected =
            " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %F{134}\u{ec19} gpt-4%f %F{3}XHIGH%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_reasoning_effort_narrow_terminal_uses_short_form() {
        // Below the wide-terminal threshold, the reasoning effort collapses
        // to the first three characters uppercased ("MEDIUM" -> "MED").
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .reasoning_effort(Some(Effort::Medium))
            .terminal_width(Some(80))
            .to_string();

        let expected =
            " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %F{134}\u{ec19} gpt-4%f %F{3}MED%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_reasoning_effort_wide_terminal_uses_full_form() {
        // At or above the wide-terminal threshold, the full uppercase label
        // is rendered (e.g. "MEDIUM" rather than "MED").
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .reasoning_effort(Some(Effort::Medium))
            .terminal_width(Some(120))
            .to_string();

        let expected =
            " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %F{134}\u{ec19} gpt-4%f %F{3}MEDIUM%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_reasoning_effort_at_threshold_is_full_form() {
        // The threshold is inclusive: a width of exactly
        // `WIDE_TERMINAL_THRESHOLD` columns renders the full label.
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .reasoning_effort(Some(Effort::High))
            .terminal_width(Some(WIDE_TERMINAL_THRESHOLD))
            .to_string();

        let expected =
            " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %F{134}\u{ec19} gpt-4%f %F{3}HIGH%f";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_reasoning_effort_short_form_minimal() {
        // The longest variant name ("MINIMAL", 7 chars) must truncate to
        // exactly three characters ("MIN") in the compact form.
        let actual = ZshRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .reasoning_effort(Some(Effort::Minimal))
            .terminal_width(Some(80))
            .to_string();

        let expected =
            " %B%F{15}\u{f167a} FORGE%f%b %B%F{15}1.5k%f%b %F{134}\u{ec19} gpt-4%f %F{3}MIN%f";
        assert_eq!(actual, expected);
    }
}
