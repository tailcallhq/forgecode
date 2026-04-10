mod util;

use std::ops::{Deref, RangeInclusive};

use util::{deref_messages, replace_range};

pub struct Compaction<Item> {
    summarize: Box<dyn Fn(&[&Item]) -> Item>,
    threshold: Box<dyn Fn(&[&Item]) -> bool>,
    retain: usize,
}

pub trait ContextMessage {
    fn is_user(&self) -> bool;
    fn is_assistant(&self) -> bool;
    fn is_system(&self) -> bool;
    fn is_toolcall(&self) -> bool;
    fn is_toolcall_result(&self) -> bool;
}

/// A compacted summary that replaces a range of original messages.
struct Summary<I> {
    /// The synthesised summary item.
    message: I,
    /// The original messages that were compacted into this summary.
    source: Vec<I>,
}

enum Message<I> {
    Summary(Summary<I>),
    Original { message: I },
}

impl<I> Message<I> {
    fn is_summary(&self) -> bool {
        matches!(self, Message::Summary(_))
    }

    fn is_original(&self) -> bool {
        matches!(self, Message::Original { .. })
    }
}

impl<I> Deref for Message<I> {
    type Target = I;

    fn deref(&self) -> &Self::Target {
        match self {
            Message::Summary(Summary { message, .. }) => message,
            Message::Original { message } => message,
        }
    }
}

impl<Item: ContextMessage + Clone> Compaction<Item> {
    pub fn compact_conversation(&self, messages: Vec<Item>) -> Vec<Item> {
        todo!()
    }

    fn threshold(&self, messages: &[Message<Item>]) -> bool {
        (self.threshold)(deref_messages(messages).as_slice())
    }

    fn summarize(&self, messages: &[Message<Item>]) -> Item {
        (self.summarize)(deref_messages(messages).as_slice())
    }

    fn compact_conversation_slice(&self, messages: Vec<Message<Item>>) -> Vec<Message<Item>> {
        if self.threshold(messages.as_slice()) {
            self.compact_complete(messages)
        } else {
            messages
        }
    }

    fn find_compact_range(&self, messages: &[Message<Item>]) -> Option<RangeInclusive<usize>> {
        if messages.is_empty() {
            return None;
        }

        let length = messages.len();

        let start = messages
            .iter()
            .enumerate()
            // Skip all summaries
            .filter(|i| i.1.is_original())
            .find(|i| i.1.is_assistant())
            .map(|i| i.0)?;

        // Don't compact if there's no assistant message
        if start >= length {
            return None;
        }

        // Calculate the end index based on preservation window
        // If we need to preserve all or more messages than we have, there's nothing to
        // compact
        if self.retain >= length {
            return None;
        }

        // Use saturating subtraction to prevent potential overflow
        let mut end = length.saturating_sub(self.retain).saturating_sub(1);

        // If start > end or end is invalid, don't compact
        if start > end || end >= length {
            return None;
        }

        // Don't break between a tool call and its result
        if messages.get(end).is_some_and(|msg| msg.is_toolcall()) {
            // If the last message has a tool call, adjust end to include the tool result
            // This means either not compacting at all, or reducing the end by 1
            if end == start {
                // If start == end and it has a tool call, don't compact
                return None;
            } else {
                // Otherwise reduce end by 1
                return Some(start..=end.saturating_sub(1));
            }
        }

        if messages
            .get(end)
            .is_some_and(|msg| msg.is_toolcall_result())
            && messages
                .get(end.saturating_add(1))
                .is_some_and(|msg| msg.is_toolcall_result())
        {
            // If the last message is a tool result and the next one is also a tool result,
            // we need to adjust the end.
            while end >= start
                && messages
                    .get(end)
                    .is_some_and(|msg| msg.is_toolcall_result())
            {
                end = end.saturating_sub(1);
            }
            end = end.saturating_sub(1);
        }

        // Return the sequence only if it has at least one message
        if end >= start {
            Some(start..=end)
        } else {
            None
        }
    }

    fn compact_complete(&self, messages: Vec<Message<Item>>) -> Vec<Message<Item>> {
        if let Some(range) = self.find_compact_range(&messages) {
            let source_slice = &messages[*range.start()..=*range.end()];
            let summary = Message::Summary(Summary {
                message: self.summarize(source_slice),
                source: source_slice.iter().map(|m| m.deref().clone()).collect(),
            });

            replace_range(messages, summary, range)
        } else {
            messages
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// A minimal message type for testing `find_compact_range`.
    #[derive(Clone, Debug, PartialEq)]
    struct TestMsg {
        role: char,
    }

    impl TestMsg {
        fn new(role: char) -> Self {
            Self { role }
        }
    }

    impl ContextMessage for TestMsg {
        fn is_user(&self) -> bool {
            self.role == 'u'
        }
        fn is_assistant(&self) -> bool {
            self.role == 'a' || self.role == 't'
        }
        fn is_system(&self) -> bool {
            self.role == 's'
        }
        fn is_toolcall(&self) -> bool {
            self.role == 't'
        }
        fn is_toolcall_result(&self) -> bool {
            self.role == 'r'
        }
    }

    fn compaction(retain: usize) -> Compaction<TestMsg> {
        Compaction {
            summarize: Box::new(|_| TestMsg::new('S')),
            threshold: Box::new(|_| true),
            retain,
        }
    }

    /// Build a `Vec<Message<TestMsg>>` from a pattern string where each char
    /// maps to a role: s=system, u=user, a=assistant, t=toolcall, r=toolcall_result.
    fn messages_from(pattern: &str) -> Vec<Message<TestMsg>> {
        pattern
            .chars()
            .map(|c| Message::Original { message: TestMsg::new(c) })
            .collect()
    }

    /// Returns the pattern string with `[` and `]` inserted around the compacted
    /// range, mirroring the helper in `forge_domain`.
    fn seq(pattern: &str, retain: usize) -> String {
        let c = compaction(retain);
        let messages = messages_from(pattern);
        let range = c.find_compact_range(&messages);

        let mut result = pattern.to_string();
        if let Some(range) = range {
            result.insert(*range.start(), '[');
            result.insert(range.end() + 2, ']');
        }
        result
    }

    #[test]
    fn test_sequence_finding() {
        // Basic compaction scenarios
        assert_eq!(seq("suaaau", 0), "su[aaau]");
        assert_eq!(seq("sua", 0), "su[a]");
        assert_eq!(seq("suauaa", 0), "su[auaa]");

        // Tool call scenarios
        assert_eq!(seq("suttu", 0), "su[ttu]");
        assert_eq!(seq("sutraau", 0), "su[traau]");
        assert_eq!(seq("utrutru", 0), "u[trutru]");
        assert_eq!(seq("uttarru", 0), "u[ttarru]");
        assert_eq!(seq("urru", 0), "urru");
        assert_eq!(seq("uturu", 0), "u[turu]");

        // Preservation window scenarios
        assert_eq!(seq("suaaaauaa", 0), "su[aaaauaa]");
        assert_eq!(seq("suaaaauaa", 3), "su[aaaa]uaa");
        assert_eq!(seq("suaaaauaa", 5), "su[aa]aauaa");
        assert_eq!(seq("suaaaauaa", 8), "suaaaauaa");
        assert_eq!(seq("suauaaa", 0), "su[auaaa]");
        assert_eq!(seq("suauaaa", 2), "su[aua]aa");
        assert_eq!(seq("suauaaa", 1), "su[auaa]a");

        // Tool call atomicity preservation
        assert_eq!(seq("sutrtrtra", 0), "su[trtrtra]");
        assert_eq!(seq("sutrtrtra", 1), "su[trtrtr]a");
        assert_eq!(seq("sutrtrtra", 2), "su[trtr]tra");

        // Parallel tool calls
        assert_eq!(seq("sutrtrtrra", 2), "su[trtr]trra");
        assert_eq!(seq("sutrtrtrra", 3), "su[trtr]trra");
        assert_eq!(seq("sutrrrrrra", 2), "sutrrrrrra");

        // Conversation patterns
        assert_eq!(seq("suauauaua", 0), "su[auauaua]");
        assert_eq!(seq("suauauaua", 2), "su[auaua]ua");
        assert_eq!(seq("suauauaua", 6), "su[a]uauaua");
        assert_eq!(seq("sutruaua", 0), "su[truaua]");
        assert_eq!(seq("sutruaua", 3), "su[tru]aua");

        // Special cases
        assert_eq!(seq("saua", 0), "s[aua]");
        assert_eq!(seq("suaut", 0), "su[au]t");

        // Edge cases
        assert_eq!(seq("", 0), "");
        assert_eq!(seq("s", 0), "s");
        assert_eq!(seq("sua", 3), "sua");
        assert_eq!(seq("ut", 0), "ut");
        assert_eq!(seq("suuu", 0), "suuu");
        assert_eq!(seq("ut", 1), "ut");
        assert_eq!(seq("ua", 0), "u[a]");
    }
}
