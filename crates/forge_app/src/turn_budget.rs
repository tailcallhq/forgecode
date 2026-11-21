use forge_domain::{InterruptionReason, ModelId, Usage};
use llm_budget::{AgentBudget, ChargeContext};

/// Hard dollar cap for one orchestrator turn, backed by `llm-budget`.
///
/// Each finished request is booked against the cap. The provider-reported
/// cost is used when the response carries one; otherwise the request is
/// priced from its token counts with llm-budget's dated price table. A
/// request that cannot be priced at all stops the turn instead of being
/// counted as free, because a cap that silently books $0 does not cap
/// anything.
pub struct TurnBudget {
    budget: AgentBudget,
}

impl TurnBudget {
    /// Creates a budget capped at `limit_usd` for the agent `agent_id`.
    ///
    /// # Errors
    /// Returns an error if `limit_usd` is not a finite positive amount.
    pub fn new(agent_id: &str, limit_usd: f64) -> anyhow::Result<Self> {
        let budget = AgentBudget::builder(agent_id, limit_usd)
            .build()
            .map_err(|e| anyhow::anyhow!("invalid max_cost_per_turn {limit_usd}: {e}"))?;
        Ok(Self { budget })
    }

    /// Books one finished request and reports whether the turn has to stop.
    ///
    /// Returns `Some` with the interruption reason when the turn has spent
    /// its cap, or when the request had no reported cost and `model` has no
    /// known price. Returns `None` while there is money left.
    ///
    /// # Arguments
    /// * `model` - The model that served the request
    /// * `usage` - The usage block of the response
    pub fn record(&self, model: &ModelId, usage: &Usage) -> Option<InterruptionReason> {
        let ctx = ChargeContext::new().model(model.as_str());
        let booked = match usage.cost {
            Some(cost) => Ok(self.budget.charge_usd_force(cost, ctx)),
            None => {
                let prompt = *usage.prompt_tokens as u64;
                let cached = (*usage.cached_tokens as u64).min(prompt);
                let tokens = llm_budget::Usage::from_parts(
                    prompt - cached,
                    cached,
                    0,
                    *usage.completion_tokens as u64,
                );
                self.budget.record_with(model.as_str(), &tokens, ctx)
            }
        };

        match booked {
            Err(e) => Some(InterruptionReason::UnpricedRequest {
                model: model.to_string(),
                reason: e.to_string(),
            }),
            Ok(_) if self.budget.spent_usd() >= self.budget.limit_usd() => {
                Some(InterruptionReason::MaxCostPerTurnLimitReached {
                    limit_usd: self.budget.limit_usd(),
                    spent_usd: self.budget.spent_usd(),
                })
            }
            Ok(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::TokenCount;
    use pretty_assertions::assert_eq;

    use super::*;

    fn fixture_usage(cost: Option<f64>) -> Usage {
        Usage {
            prompt_tokens: TokenCount::Actual(10_000),
            completion_tokens: TokenCount::Actual(500),
            total_tokens: TokenCount::Actual(10_500),
            cached_tokens: TokenCount::Actual(0),
            cost,
        }
    }

    #[test]
    fn test_reported_cost_stops_turn_at_cap() {
        let fixture = TurnBudget::new("forge", 0.05).unwrap();
        let model = ModelId::new("openrouter/some-model");

        let first = fixture.record(&model, &fixture_usage(Some(0.03)));
        let actual = fixture.record(&model, &fixture_usage(Some(0.03)));

        assert_eq!(first, None);
        assert!(matches!(
            actual,
            Some(InterruptionReason::MaxCostPerTurnLimitReached { limit_usd, spent_usd })
                if (limit_usd - 0.05).abs() < 1e-9 && (spent_usd - 0.06).abs() < 1e-9
        ));
    }

    #[test]
    fn test_unreported_cost_is_priced_from_tokens() {
        // Direct Anthropic/OpenAI responses carry no cost field. Without a
        // price table the cap would book $0 and never fire.
        let fixture = TurnBudget::new("forge", 1.0).unwrap();
        let model = ModelId::new("gpt-4o");

        let actual = fixture.record(&model, &fixture_usage(None));

        assert_eq!(actual, None);
        // gpt-4o: 10k input at $2.50/M + 500 output at $10/M = $0.03
        assert!((fixture.budget.spent_usd() - 0.03).abs() < 1e-9);
    }

    #[test]
    fn test_unknown_model_without_cost_fails_closed() {
        let fixture = TurnBudget::new("forge", 1.0).unwrap();
        let model = ModelId::new("no-such-model-x");

        let actual = fixture.record(&model, &fixture_usage(None));

        assert!(matches!(
            actual,
            Some(InterruptionReason::UnpricedRequest { ref model, .. }) if model == "no-such-model-x"
        ));
        assert_eq!(fixture.budget.spent_usd(), 0.0);
    }

    #[test]
    fn test_invalid_limit_is_rejected() {
        let actual = TurnBudget::new("forge", -1.0).is_err();
        assert_eq!(actual, true);
    }
}
