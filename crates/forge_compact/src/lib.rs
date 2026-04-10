mod util;

use std::ops::{Deref, RangeInclusive};

use util::replace_range;

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

enum Message<I> {
    // FIXME: Create a new type for Summary {message, range} and use that type in Message::Summary
    Summary { message: I, source: Vec<I> },
    Original { message: I },
}

impl<I> Message<I> {
    fn is_summary(&self) -> bool {
        todo!()
    }

    fn is_original(&self) -> bool {
        todo!()
    }
}

impl<I> Deref for Message<I> {
    type Target = I;

    fn deref(&self) -> &Self::Target {
        match self {
            Message::Summary { message, .. } => message,
            Message::Original { message } => message,
        }
    }
}

impl<Item: ContextMessage> Compaction<Item> {
    pub fn compact_conversation(&self, messages: Vec<Item>) -> Vec<Item> {
        todo!()
    }

    fn threshold(&self, messages: &[Message<Item>]) -> bool {
        (self.threshold)(
            // FIXME: Create a helper for this conversion in utils and use it
            messages
                .iter()
                .map(|item| item.deref())
                .collect::<Vec<_>>()
                .as_slice(),
        )
    }

    fn summarize(&self, messages: &[Message<Item>]) -> Item {
        (self.summarize)(
            // FIXME: Create a helper for this conversion in utils and use it
            messages
                .iter()
                .map(|item| item.deref())
                .collect::<Vec<_>>()
                .as_slice(),
        )
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
            let summary = Message::Summary {
                message: self.summarize(&messages[*range.start()..=*range.end()]),
                // FIXME: Add the selected message range
                source: Vec::new(),
            };

            replace_range(messages, summary, range)
        } else {
            messages
        }
    }
}

#[cfg(test)]
mod tests {
    // FIXME: Add forge_domain/src/compact/strategy.rs::test_sequence_finding tests
}
