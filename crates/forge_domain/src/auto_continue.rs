use forge_domain::FinishReason;

#[derive(Debug, Clone)]
pub struct AutoContinueConfig {
    pub confidence_threshold: u8,
    pub max_auto_continues: usize,
    pub intent_phrases: Vec<String>,
    pub completion_phrases: Vec<String>,
    pub summary_phrases: Vec<String>,
}

impl Default for AutoContinueConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: 60,
            max_auto_continues: 3,
            intent_phrases: vec![
                "let me continue".into(),
                "i'll continue".into(),
                "i will continue".into(),
                "now i'll".into(),
                "next, i'll".into(),
                "moving on to".into(),
                "proceed with".into(),
                "now let me".into(),
                "i'll now".into(),
                "next step".into(),
                "continuing with".into(),
                "now i need to".into(),
                "i still need to".into(),
                "remaining steps".into(),
            ],
            completion_phrases: vec![
                "task is complete".into(),
                "i'm done".into(),
                "all changes have been made".into(),
                "the implementation is complete".into(),
                "finished implementing".into(),
                "all files have been".into(),
                "everything is now".into(),
                "successfully completed".into(),
                "i have completed".into(),
                "the bug is fixed".into(),
            ],
            summary_phrases: vec![
                "to summarize".into(),
                "in summary".into(),
                "here's a summary".into(),
                "let me summarize".into(),
                "overview of changes".into(),
                "here's what was done".into(),
                "recap of".into(),
                "let me know if".into(),
                "when you're ready".into(),
                "waiting for your".into(),
                "please review".into(),
                "let me know if you'd like".into(),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct AutoContinueDecision {
    pub should_continue: bool,
    pub confidence: u8,
    pub reason: String,
    pub signals: Vec<SignalResult>,
}

#[derive(Debug, Clone)]
pub struct SignalResult {
    pub name: String,
    pub triggered: bool,
    pub score: u8,
    pub detail: String,
}

pub struct AutoContinueAnalyzer {
    config: AutoContinueConfig,
}

impl AutoContinueAnalyzer {
    pub fn new(config: AutoContinueConfig) -> Self {
        Self { config }
    }

    pub fn analyze(
        &self,
        content: &Option<String>,
        finish_reason: &Option<FinishReason>,
        last_event_was_tool_result: bool,
        recent_tool_call_ratio: f64,
    ) -> AutoContinueDecision {
        let mut signals = Vec::new();
        let mut total_score: u8 = 0;

        let s1 = self.analyze_finish_reason(finish_reason);
        total_score += s1.score;
        signals.push(s1);

        let s2 = self.analyze_last_event(last_event_was_tool_result);
        total_score += s2.score;
        signals.push(s2);

        let s3 = self.analyze_content_intent(content);
        total_score += s3.score;
        signals.push(s3);

        let s4 = self.analyze_summary_language(content);
        total_score += s4.score;
        signals.push(s4);

        let s5 = self.analyze_tool_call_ratio(recent_tool_call_ratio);
        total_score += s5.score;
        signals.push(s5);

        let should_continue = total_score >= self.config.confidence_threshold;

        let triggered_signals: Vec<&str> = signals
            .iter()
            .filter(|s| s.triggered)
            .map(|s| s.name.as_str())
            .collect();

        let reason = if should_continue {
            format!(
                "Auto-continue: score {} >= threshold {} (signals: {})",
                total_score,
                self.config.confidence_threshold,
                triggered_signals.join(" + ")
            )
        } else {
            format!(
                "Finish turn: score {} < threshold {}",
                total_score, self.config.confidence_threshold
            )
        };

        AutoContinueDecision {
            should_continue,
            confidence: total_score,
            reason,
            signals,
        }
    }

    fn analyze_finish_reason(&self, finish_reason: &Option<FinishReason>) -> SignalResult {
        match finish_reason {
            Some(FinishReason::ToolCalls) => SignalResult {
                name: "finish_reason=tool_calls".into(),
                triggered: true,
                score: 30,
                detail: "Model indicated tool use but didn't provide tool_calls".into(),
            },
            Some(FinishReason::Stop) => SignalResult {
                name: "finish_reason=stop".into(),
                triggered: false,
                score: 0,
                detail: "Model explicitly stopped".into(),
            },
            Some(FinishReason::ContentFilter) => SignalResult {
                name: "finish_reason=content_filter".into(),
                triggered: false,
                score: 0,
                detail: "Content was filtered".into(),
            },
            Some(FinishReason::Length) => SignalResult {
                name: "finish_reason=length".into(),
                triggered: false,
                score: 0,
                detail: "Response hit length limit".into(),
            },
            None => SignalResult {
                name: "finish_reason=none".into(),
                triggered: false,
                score: 15,
                detail: "No finish_reason provided".into(),
            },
        }
    }

    fn analyze_last_event(&self, last_was_tool_result: bool) -> SignalResult {
        if last_was_tool_result {
            SignalResult {
                name: "last_event=ToolResult".into(),
                triggered: true,
                score: 25,
                detail: "Last event was a tool result - model should continue".into(),
            }
        } else {
            SignalResult {
                name: "last_event=not_tool_result".into(),
                triggered: false,
                score: 0,
                detail: "Last event was not a tool result".into(),
            }
        }
    }

    fn analyze_content_intent(&self, content: &Option<String>) -> SignalResult {
        let Some(content) = content else {
            return SignalResult {
                name: "content_intent".into(),
                triggered: false,
                score: 0,
                detail: "No content to analyze".into(),
            };
        };

        let lower = content.to_lowercase();

        let has_completion = self.config.completion_phrases.iter()
            .any(|phrase| lower.contains(&phrase.to_lowercase()));

        if has_completion {
            return SignalResult {
                name: "content_intent".into(),
                triggered: false,
                score: 0,
                detail: "Content contains COMPLETION phrases - task likely done".into(),
            };
        }

        let has_intent = self.config.intent_phrases.iter()
            .any(|phrase| lower.contains(&phrase.to_lowercase()));

        if has_intent {
            SignalResult {
                name: "content_intent".into(),
                triggered: true,
                score: 25,
                detail: "Content contains intent phrases - model wants to continue".into(),
            }
        } else {
            SignalResult {
                name: "content_intent".into(),
                triggered: false,
                score: 0,
                detail: "No intent or completion phrases found".into(),
            }
        }
    }

    fn analyze_summary_language(&self, content: &Option<String>) -> SignalResult {
        let Some(content) = content else {
            return SignalResult {
                name: "no_summary_language".into(),
                triggered: true,
                score: 10,
                detail: "No content - no summary language".into(),
            };
        };

        let lower = content.to_lowercase();
        let has_summary = self.config.summary_phrases.iter()
            .any(|phrase| lower.contains(&phrase.to_lowercase()));

        if has_summary {
            SignalResult {
                name: "no_summary_language".into(),
                triggered: false,
                score: 0,
                detail: "Content contains SUMMARY phrases - model is summarizing".into(),
            }
        } else {
            SignalResult {
                name: "no_summary_language".into(),
                triggered: true,
                score: 10,
                detail: "No summary language detected".into(),
            }
        }
    }

    fn analyze_tool_call_ratio(&self, ratio: f64) -> SignalResult {
        if ratio > 0.5 {
            SignalResult {
                name: "tool_call_ratio".into(),
                triggered: true,
                score: 10,
                detail: format!("High tool call ratio ({:.0}%) - likely in multi-step task", ratio * 100.0),
            }
        } else {
            SignalResult {
                name: "tool_call_ratio".into(),
                triggered: false,
                score: 0,
                detail: format!("Low tool call ratio ({:.0}%) - likely not in multi-step task", ratio * 100.0),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyzer() -> AutoContinueAnalyzer {
        AutoContinueAnalyzer::new(AutoContinueConfig::default())
    }

    #[test]
    fn test_model_says_continue_after_tool_result() {
        let analyzer = analyzer();
        let decision = analyzer.analyze(
            &Some("I've written the file. Let me continue with the next step.".into()),
            &Some(FinishReason::Stop),
            true,
            0.75,
        );
        assert!(decision.should_continue, "Should auto-continue: {}", decision.reason);
        assert!(decision.confidence >= 60);
    }

    #[test]
    fn test_finish_reason_tool_calls_but_empty() {
        let analyzer = analyzer();
        let decision = analyzer.analyze(
            &None,
            &Some(FinishReason::ToolCalls),
            true,
            0.80,
        );
        assert!(decision.should_continue, "Should auto-continue: {}", decision.reason);
    }

    #[test]
    fn test_no_finish_reason_after_tool_result() {
        let analyzer = analyzer();
        let decision = analyzer.analyze(
            &Some("Now I need to fix the imports.".into()),
            &None,
            true,
            0.60,
        );
        assert!(decision.should_continue, "Should auto-continue: {}", decision.reason);
    }

    #[test]
    fn test_task_complete_with_summary() {
        let analyzer = analyzer();
        let decision = analyzer.analyze(
            &Some("The implementation is complete. To summarize, I fixed the bug by...".into()),
            &Some(FinishReason::Stop),
            true,
            0.50,
        );
        assert!(!decision.should_continue, "Should NOT auto-continue: {}", decision.reason);
    }

    #[test]
    fn test_model_waiting_for_user() {
        let analyzer = analyzer();
        let decision = analyzer.analyze(
            &Some("Let me know if you'd like me to make any changes.".into()),
            &Some(FinishReason::Stop),
            false,
            0.30,
        );
        assert!(!decision.should_continue, "Should NOT auto-continue: {}", decision.reason);
    }

    #[test]
    fn test_bug_fixed_summary() {
        let analyzer = analyzer();
        let decision = analyzer.analyze(
            &Some("The bug is fixed. All changes have been made successfully.".into()),
            &Some(FinishReason::Stop),
            true,
            0.60,
        );
        assert!(!decision.should_continue, "Should NOT auto-continue: {}", decision.reason);
    }

    #[test]
    fn test_empty_content_after_tool_result() {
        let analyzer = analyzer();
        let decision = analyzer.analyze(
            &None,
            &Some(FinishReason::Stop),
            true,
            0.50,
        );
        assert!(!decision.should_continue);
    }

    #[test]
    fn test_intent_but_no_tool_history() {
        let analyzer = analyzer();
        let decision = analyzer.analyze(
            &Some("Let me continue with the implementation.".into()),
            &Some(FinishReason::Stop),
            false,
            0.20,
        );
        assert!(!decision.should_continue);
    }
}
