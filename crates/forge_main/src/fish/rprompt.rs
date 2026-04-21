//! Fish right prompt implementation.
//!
//! Provides the right prompt display for the Fish shell integration,
//! showing agent name, model, and token count information.
//! Uses ANSI escape codes instead of ZSH prompt escapes.

use std::fmt::{self, Display};

use convert_case::{Case, Casing};
use derive_setters::Setters;
use forge_config::ForgeConfig;
use forge_domain::{AgentId, Effort, ModelId, TokenCount};

use crate::utils::humanize_number;

/// ANSI 256-color foreground escape: `\x1b[38;5;{N}m`.
struct AnsiColor(u8);

impl AnsiColor {
    /// White (color 15).
    const WHITE: Self = Self(15);
    /// Cyan (color 134).
    const CYAN: Self = Self(134);
    /// Green (color 2).
    const GREEN: Self = Self(2);
    /// Yellow (color 214).
    const YELLOW: Self = Self(214);
    /// Dimmed gray (color 240).
    const DIMMED: Self = Self(240);
}

/// A styled string fragment using ANSI escape codes.
struct AnsiStyled<'a> {
    text: &'a str,
    fg: Option<AnsiColor>,
    bold: bool,
}

impl<'a> AnsiStyled<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, fg: None, bold: false }
    }

    fn fg(mut self, color: AnsiColor) -> Self {
        self.fg = Some(color);
        self
    }

    fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
}

impl Display for AnsiStyled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.bold {
            write!(f, "\x1b[1m")?;
        }
        if let Some(ref color) = self.fg {
            write!(f, "\x1b[38;5;{}m", color.0)?;
        }
        write!(f, "{}", self.text)?;
        // Reset all attributes
        if self.fg.is_some() || self.bold {
            write!(f, "\x1b[0m")?;
        }
        Ok(())
    }
}

trait AnsiStyle {
    fn ansi(&self) -> AnsiStyled<'_>;
}

impl AnsiStyle for str {
    fn ansi(&self) -> AnsiStyled<'_> {
        AnsiStyled::new(self)
    }
}

impl AnsiStyle for String {
    fn ansi(&self) -> AnsiStyled<'_> {
        AnsiStyled::new(self.as_str())
    }
}

/// Fish right prompt displaying agent, model, and token count.
///
/// Formats shell prompt information with appropriate ANSI colors:
/// - Inactive state (no tokens): dimmed colors
/// - Active state (has tokens): bright white/cyan colors
#[derive(Setters)]
pub struct FishRPrompt {
    agent: Option<AgentId>,
    model: Option<ModelId>,
    token_count: Option<TokenCount>,
    cost: Option<f64>,
    context_length: Option<u64>,
    effort: Option<Effort>,
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

impl FishRPrompt {
    /// Constructs a [`FishRPrompt`] with currency settings populated from the
    /// provided [`ForgeConfig`].
    pub fn from_config(config: &ForgeConfig) -> Self {
        Self::default()
            .currency_symbol(config.currency_symbol.clone())
            .conversion_ratio(config.currency_conversion_rate.value())
    }
}

impl Default for FishRPrompt {
    fn default() -> Self {
        Self {
            agent: None,
            model: None,
            token_count: None,
            cost: None,
            context_length: None,
            effort: None,
            use_nerd_font: true,
            currency_symbol: "\u{f155}".to_string(),
            conversion_ratio: 1.0,
        }
    }
}

const AGENT_SYMBOL: &str = "\u{f167a}";
const MODEL_SYMBOL: &str = "\u{ec19}";

impl Display for FishRPrompt {
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
            agent_id.ansi().bold().fg(AnsiColor::WHITE)
        } else {
            agent_id.ansi().bold().fg(AnsiColor::DIMMED)
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
                let mut token_str = format!("{}{}", prefix, num);
                if let Some(limit) = self.context_length
                    && limit > 0
                {
                    let pct = (*count * 100).checked_div(limit as usize).unwrap_or(0);
                    token_str.push_str(&format!(" ({}%)", pct));
                }
                write!(f, " {}", token_str.ansi().fg(AnsiColor::WHITE).bold())?;
            }
        }

        // Add cost
        if let Some(cost) = self.cost
            && active
        {
            let converted_cost = cost * self.conversion_ratio;
            let cost_str = format!("{}{:.2}", self.currency_symbol, converted_cost);
            write!(f, " {}", cost_str.ansi().fg(AnsiColor::GREEN).bold())?;
        }

        // Add effort
        if let Some(ref effort) = self.effort {
            let styled = if active {
                effort.short_name().ansi().fg(AnsiColor::YELLOW).bold()
            } else {
                effort.short_name().ansi().fg(AnsiColor::DIMMED)
            };
            write!(f, " [{}]", styled)?;
        }

        // Add model (always colored — it's a static config identifier, not
        // conversation state)
        if let Some(ref model_id) = self.model {
            let model_id = if self.use_nerd_font {
                format!("{MODEL_SYMBOL} {}", model_id)
            } else {
                model_id.to_string()
            };
            write!(f, " {}", model_id.ansi().fg(AnsiColor::CYAN))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rprompt_init_state() {
        let actual = FishRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .to_string();

        let expected =
            " \x1b[1m\x1b[38;5;240m\u{f167a} FORGE\x1b[0m \x1b[38;5;134m\u{ec19} gpt-4\x1b[0m";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_tokens() {
        let actual = FishRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .to_string();

        let expected = " \x1b[1m\x1b[38;5;15m\u{f167a} FORGE\x1b[0m \x1b[1m\x1b[38;5;15m1.5k\x1b[0m \x1b[38;5;134m\u{ec19} gpt-4\x1b[0m";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_tokens_and_cost() {
        let actual = FishRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .cost(Some(0.0123))
            .currency_symbol("\u{f155}")
            .to_string();

        let expected = " \x1b[1m\x1b[38;5;15m\u{f167a} FORGE\x1b[0m \x1b[1m\x1b[38;5;15m1.5k\x1b[0m \x1b[1m\x1b[38;5;2m\u{f155}0.01\x1b[0m \x1b[38;5;134m\u{ec19} gpt-4\x1b[0m";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_without_nerdfonts() {
        let actual = FishRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .use_nerd_font(false)
            .to_string();

        let expected = " \x1b[1m\x1b[38;5;15mFORGE\x1b[0m \x1b[1m\x1b[38;5;15m1.5k\x1b[0m \x1b[38;5;134mgpt-4\x1b[0m";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_currency_conversion() {
        let actual = FishRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .model(Some(ModelId::new("gpt-4")))
            .token_count(Some(TokenCount::Actual(1500)))
            .cost(Some(0.01))
            .currency_symbol("INR")
            .conversion_ratio(83.5)
            .to_string();

        let expected = " \x1b[1m\x1b[38;5;15m\u{f167a} FORGE\x1b[0m \x1b[1m\x1b[38;5;15m1.5k\x1b[0m \x1b[1m\x1b[38;5;2mINR0.83\x1b[0m \x1b[38;5;134m\u{ec19} gpt-4\x1b[0m";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_rprompt_with_context_percentage() {
        let actual = FishRPrompt::default()
            .agent(Some(AgentId::new("forge")))
            .token_count(Some(TokenCount::Actual(15000)))
            .context_length(Some(100000))
            .use_nerd_font(false)
            .to_string();

        assert!(actual.contains("15k (15%)"));
    }
}
