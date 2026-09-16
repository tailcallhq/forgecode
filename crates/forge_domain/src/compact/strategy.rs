use crate::{Context, Role};

/// Strategy for context compaction that unifies different compaction approaches
#[derive(Debug, Clone)]
pub enum CompactionStrategy {
    /// Retention based on percentage of tokens
    Evict(f64),
    /// Retention based on fixed tokens
    Retain(usize),

    /// Selects the strategy with minimum retention
    Min(Box<CompactionStrategy>, Box<CompactionStrategy>),

    /// Selects the strategy with maximum retention
    Max(Box<CompactionStrategy>, Box<CompactionStrategy>),
}

impl CompactionStrategy {
    /// Create a percentage-based compaction strategy
    pub fn evict(percentage: f64) -> Self {
        Self::Evict(percentage)
    }

    /// Create a preserve-last-N compaction strategy
    pub fn retain(preserve_last_n: usize) -> Self {
        Self::Retain(preserve_last_n)
    }

    pub fn min(self, other: CompactionStrategy) -> Self {
        CompactionStrategy::Min(Box::new(self), Box::new(other))
    }

    pub fn max(self, other: CompactionStrategy) -> Self {
        CompactionStrategy::Max(Box::new(self), Box::new(other))
    }

    /// Convert percentage-based strategy to preserve_last_n equivalent
    /// This simulates the original percentage algorithm to determine how many
    /// messages would be preserved, then returns that as a preserve_last_n
    /// value
    fn to_fixed(&self, context: &Context) -> usize {
        match self {
            CompactionStrategy::Evict(percentage) => {
                let percentage = percentage.min(1.0);
                let total_tokens = context.token_count();
                let mut eviction_budget: usize =
                    (percentage * (*total_tokens) as f64).ceil() as usize;

                let range = context
                    .messages
                    .iter()
                    .enumerate()
                    // Skip system message
                    .filter(|m| !m.1.has_role(Role::System))
                    .find(|(_, m)| {
                        eviction_budget = eviction_budget.saturating_sub(m.token_count_approx());
                        eviction_budget == 0
                    });

                match range {
                    Some((i, _)) => i,
                    None => context.messages.len().saturating_sub(1),
                }
            }
            CompactionStrategy::Retain(fixed) => *fixed,
            CompactionStrategy::Min(a, b) => a.to_fixed(context).min(b.to_fixed(context)),
            CompactionStrategy::Max(a, b) => a.to_fixed(context).max(b.to_fixed(context)),
        }
    }

    /// Find the sequence to compact using the unified algorithm
    pub fn eviction_range(&self, context: &Context) -> Option<(usize, usize)> {
        let retention = self.to_fixed(context);
        find_sequence_preserving_last_n(context, retention)
    }
}

/// Finds a sequence in the context for compaction, starting from the first
/// assistant message and including all messages up to the last possible message
/// (respecting preservation window)
fn find_sequence_preserving_last_n(
    context: &Context,
    max_retention: usize,
) -> Option<(usize, usize)> {
    let messages = &context.messages;
    if messages.is_empty() {
        return None;
    }

    // len will be always > 0
    let length = messages.len();

    // Find the first assistant message index
    let start = messages
        .iter()
        .enumerate()
        .find(|(_, message)| message.has_role(Role::Assistant))
        .map(|(index, _)| index)?;

    // Don't compact if there's no assistant message
    if start >= length {
        return None;
    }

    // Calculate the end index based on preservation window
    // If we need to preserve all or more messages than we have, there's nothing
    // to compact
    if max_retention >= length {
        return None;
    }

    // Use saturating subtraction to prevent potential overflow
    let mut end = length.saturating_sub(max_retention).saturating_sub(1);

    // If start > end or end is invalid, don't compact
    if start > end || end >= length {
        return None;
    }

    // Don't break between a tool call and its result
    if messages.get(end).is_some_and(|msg| msg.has_tool_call()) {
        // If the last message has a tool call, adjust end to include the tool
        // result This means either not compacting at all, or reducing
        // the end by 1
        if end == start {
            // If start == end and it has a tool call, don't compact
            return None;
        } else {
            // Otherwise reduce end by 1
            return Some((start, end.saturating_sub(1)));
        }
    }

    if messages.get(end).is_some_and(|msg| msg.has_tool_result())
        && messages
            .get(end.saturating_add(1))
            .is_some_and(|msg| msg.has_tool_result())
    {
        // If the last message is a tool result and the next one is also a tool
        // result, we need to adjust the end.
        while end >= start && messages.get(end).is_some_and(|msg| msg.has_tool_result()) {
            end = end.saturating_sub(1);
        }
        end = end.saturating_sub(1);
    }

    // Return the sequence only if it has at least one message
    if end >= start {
        Some((start, end))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::MessagePattern;

    fn context_from_pattern(pattern: impl ToString) -> Context {
        MessagePattern::new(pattern.to_string()).build()
    }

    fn seq(pattern: impl ToString, preserve_last_n: usize) -> String {
        let pattern = pattern.to_string();
        let context = context_from_pattern(&pattern);

        let sequence = find_sequence_preserving_last_n(&context, preserve_last_n);

        let mut result = pattern.clone();
        if let Some((start, end)) = sequence {
            result.insert(start, '[');
            result.insert(end + 2, ']');
        }

        result
    }

    #[test]
    fn test_sequence_finding() {
        // Basic compaction scenarios
        let actual = seq("suaaau", 0);
        let expected = "su[aaau]";
        assert_eq!(actual, expected);

        let actual = seq("sua", 0);
        let expected = "su[a]";
        assert_eq!(actual, expected);

        let actual = seq("suauaa", 0);
        let expected = "su[auaa]";
        assert_eq!(actual, expected);

        // Tool call scenarios
        let actual = seq("suttu", 0);
        let expected = "su[ttu]";
        assert_eq!(actual, expected);

        let actual = seq("sutraau", 0);
        let expected = "su[traau]";
        assert_eq!(actual, expected);

        let actual = seq("utrutru", 0);
        let expected = "u[trutru]";
        assert_eq!(actual, expected);

        let actual = seq("uttarru", 0);
        let expected = "u[ttarru]";
        assert_eq!(actual, expected);

        let actual = seq("urru", 0);
        let expected = "urru";
        assert_eq!(actual, expected);

        let actual = seq("uturu", 0);
        let expected = "u[turu]";
        assert_eq!(actual, expected);

        // Preservation window scenarios
        let actual = seq("suaaaauaa", 0);
        let expected = "su[aaaauaa]";
        assert_eq!(actual, expected);

        let actual = seq("suaaaauaa", 3);
        let expected = "su[aaaa]uaa";
        assert_eq!(actual, expected);

        let actual = seq("suaaaauaa", 5);
        let expected = "su[aa]aauaa";
        assert_eq!(actual, expected);

        let actual = seq("suaaaauaa", 8);
        let expected = "suaaaauaa";
        assert_eq!(actual, expected);

        let actual = seq("suauaaa", 0);
        let expected = "su[auaaa]";
        assert_eq!(actual, expected);

        let actual = seq("suauaaa", 2);
        let expected = "su[aua]aa";
        assert_eq!(actual, expected);

        let actual = seq("suauaaa", 1);
        let expected = "su[auaa]a";
        assert_eq!(actual, expected);

        // Tool call atomicity preservation
        let actual = seq("sutrtrtra", 0);
        let expected = "su[trtrtra]";
        assert_eq!(actual, expected);

        let actual = seq("sutrtrtra", 1);
        let expected = "su[trtrtr]a";
        assert_eq!(actual, expected);

        let actual = seq("sutrtrtra", 2);
        let expected = "su[trtr]tra";
        assert_eq!(actual, expected);

        // Parallel tool calls
        let actual = seq("sutrtrtrra", 2);
        let expected = "su[trtr]trra";
        assert_eq!(actual, expected);

        let actual = seq("sutrtrtrra", 3);
        let expected = "su[trtr]trra";
        assert_eq!(actual, expected);

        let actual = seq("sutrrtrrtrra", 5);
        let expected = "su[trr]trrtrra";
        assert_eq!(actual, expected);

        let actual = seq("sutrrrrrra", 2);
        let expected = "sutrrrrrra"; // No compaction due to tool preservation logic
        assert_eq!(actual, expected);

        // Conversation patterns
        let actual = seq("suauauaua", 0);
        let expected = "su[auauaua]";
        assert_eq!(actual, expected);

        let actual = seq("suauauaua", 2);
        let expected = "su[auaua]ua";
        assert_eq!(actual, expected);

        let actual = seq("suauauaua", 6);
        let expected = "su[a]uauaua";
        assert_eq!(actual, expected);

        let actual = seq("sutruaua", 0);
        let expected = "su[truaua]";
        assert_eq!(actual, expected);

        let actual = seq("sutruaua", 3);
        let expected = "su[tru]aua";
        assert_eq!(actual, expected);

        // Special cases
        let actual = seq("saua", 0);
        let expected = "s[aua]";
        assert_eq!(actual, expected);

        let actual = seq("suaut", 0);
        let expected = "su[au]t";
        assert_eq!(actual, expected);

        // Edge cases
        let actual = seq("", 0);
        let expected = "";
        assert_eq!(actual, expected);

        let actual = seq("s", 0);
        let expected = "s";
        assert_eq!(actual, expected);

        let actual = seq("sua", 3);
        let expected = "sua";
        assert_eq!(actual, expected);

        let actual = seq("ut", 0);
        let expected = "ut"; // No compaction due to tool preservation
        assert_eq!(actual, expected);

        let actual = seq("suuu", 0);
        let expected = "suuu"; // No assistant messages, so no compaction
        assert_eq!(actual, expected);

        let actual = seq("ut", 1);
        let expected = "ut";
        assert_eq!(actual, expected);

        let actual = seq("ua", 0);
        let expected = "u[a]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compact_strategy_to_fixed_conversion() {
        // Create a simple context using 'sua' DSL: system, user, assistant
        let fixture = context_from_pattern("sua");

        // Test Percentage strategy conversion
        // Context: System (3 tokens), User (3 tokens), Assistant (3 tokens) = 9
        // total tokens Eviction budget: 40% of 9 = 3.6 → 4 tokens
        // (rounded up) Strategy skips system messages, so calculation
        // for non-system messages:
        // - User message (index 1): 3 tokens → budget: 4 - 3 = 1 token
        //   remaining
        // - Assistant message (index 2): 3 tokens → budget: 1 - 3 = 0
        //   (saturating_sub)
        // Result: Eviction budget exhausted at index 2 (Assistant), so to_fixed
        // returns 2
        let percentage_strategy = CompactionStrategy::evict(0.4);
        let actual = percentage_strategy.to_fixed(&fixture);
        let expected = 2;
        assert_eq!(actual, expected);

        // Test PreserveLastN strategy
        let preserve_strategy = CompactionStrategy::retain(3);
        let actual = preserve_strategy.to_fixed(&fixture);
        let expected = 3;
        assert_eq!(actual, expected);

        // Test invalid percentage (gets clamped to 1.0 = 100%)
        // With 100% eviction budget (9 tokens), we can evict all messages
        // With 9 tokens budget, all 3 messages (3+3+3) exhaust the budget at
        // message index 2
        let invalid_strategy = CompactionStrategy::evict(1.5);
        let actual = invalid_strategy.to_fixed(&fixture);
        let expected = 2; // Returns index 2 (last message) when all messages fit in budget
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compact_strategy_conversion_equivalence() {
        // Create context using DSL: user, assistant, user, assistant, user
        let fixture = context_from_pattern("uauau");

        let percentage_strategy = CompactionStrategy::evict(0.6);
        let actual_sequence = percentage_strategy.eviction_range(&fixture);

        // Convert percentage to preserve_last_n and test equivalence
        let preserve_last_n = percentage_strategy.to_fixed(&fixture);
        let preserve_strategy = CompactionStrategy::retain(preserve_last_n);
        let expected_sequence = preserve_strategy.eviction_range(&fixture);
        assert_eq!(actual_sequence, expected_sequence);
    }

    #[test]
    fn test_compact_strategy_api_usage_example() {
        // Create context using DSL: user, assistant, user, assistant
        let fixture = context_from_pattern("uaua");

        // Use percentage-based strategy
        let percentage_strategy = CompactionStrategy::evict(0.4);
        percentage_strategy.to_fixed(&fixture);

        // Use fixed window strategy - preserve last 1 message, starting from
        // first assistant
        let preserve_strategy = CompactionStrategy::retain(1);
        let actual_sequence = preserve_strategy.eviction_range(&fixture);
        let expected = Some((1, 2)); // Start from first assistant at index 1
        assert_eq!(actual_sequence, expected);
    }

    #[test]
    fn test_empty_context_no_overflow() {
        // Test that empty context doesn't cause overflow
        let empty_context = Context::default();

        let percentage_strategy = CompactionStrategy::evict(0.4);
        let actual = percentage_strategy.to_fixed(&empty_context);
        let expected = 0; // Should be 0 for empty context (saturating_sub(1) on 0 = 0)
        assert_eq!(actual, expected);

        let actual_range = percentage_strategy.eviction_range(&empty_context);
        assert_eq!(actual_range, None); // Should return None for empty context
    }

    #[test]
    fn test_single_message_context_no_overflow() {
        // Test that single message context doesn't cause overflow
        let single_context = context_from_pattern("s");

        let percentage_strategy = CompactionStrategy::evict(0.4);
        let actual = percentage_strategy.to_fixed(&single_context);
        let expected = 0; // Should be 0 (1 - 1 = 0 with saturating_sub)
        assert_eq!(actual, expected);

        let actual_range = percentage_strategy.eviction_range(&single_context);
        assert_eq!(actual_range, None); // Should return None for single system message
    }
}
