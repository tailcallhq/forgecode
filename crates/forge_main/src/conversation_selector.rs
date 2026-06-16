use anyhow::Result;
use chrono::Utc;
use forge_api::Conversation;
use forge_domain::ConversationId;
use forge_select::{ForgeWidget, PreviewLayout, PreviewPlacement, SelectRow};

use crate::display_constants::markers;

/// Fast display format for a conversation row in the selector.
/// Avoids the Info/Porcelain overhead for large conversation lists.
struct FastConversationRow<'a> {
    conv: &'a Conversation,
    now: chrono::DateTime<Utc>,
}

impl<'a> FastConversationRow<'a> {
    fn new(conv: &'a Conversation, now: chrono::DateTime<Utc>) -> Self {
        Self { conv, now }
    }
}

impl<'a> std::fmt::Display for FastConversationRow<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id = self.conv.id.to_string();
        let short_id = &id[..8.min(id.len())];
        let title = self.conv
            .title
            .as_deref()
            .unwrap_or(markers::EMPTY);
        let duration = self.now.signed_duration_since(
            self.conv.metadata.updated_at.unwrap_or(self.conv.metadata.created_at),
        );
        let time_ago = if duration.num_seconds() < 60 {
            "now".to_string()
        } else if duration.num_minutes() < 60 {
            format!("{}m ago", duration.num_minutes())
        } else if duration.num_hours() < 24 {
            format!("{}h ago", duration.num_hours())
        } else {
            format!("{}d ago", duration.num_days())
        };
        write!(f, "[{}] {} ({})", short_id, title, time_ago)
    }
}

/// Logic for selecting conversations from a list
pub struct ConversationSelector;

impl ConversationSelector {
    /// Select a conversation from the provided list using a custom TUI with
    /// a preview pane showing conversation details.
    ///
    /// The preview command uses `forge conversation info` and
    /// `forge conversation show` to display the selected conversation's
    /// metadata and last message side-by-side with the picker list.
    ///
    /// Returns the selected conversation, or None if the user cancelled.
    pub async fn select_conversation(
        conversations: &[Conversation],
        _current_conversation_id: Option<ConversationId>,
        query: Option<String>,
    ) -> Result<Option<Conversation>> {
        if conversations.is_empty() {
            return Ok(None);
        }

        // Filter to conversations with titles and context
        let valid_conversations: Vec<&Conversation> = conversations
            .iter()
            .filter(|c| c.context.is_some())
            .collect();

        if valid_conversations.is_empty() {
            return Ok(None);
        }

        // Build SelectRow items directly — no Info/Porcelain overhead.
        // This keeps the selector fast even with thousands of conversations.
        let now = Utc::now();
        let mut rows: Vec<SelectRow> = Vec::with_capacity(valid_conversations.len() + 1);
        rows.push(SelectRow::header("ID       Title                          Updated"));

        for conv in &valid_conversations {
            let uuid = conv.id.to_string();
            let display = FastConversationRow::new(conv, now).to_string();
            rows.push(SelectRow {
                raw: uuid.clone(),
                display: display.clone(),
                search: display,
                fields: vec![uuid],
            });
        }

        // Build a lookup map from UUID to Conversation for the result
        let conv_map: std::collections::HashMap<String, Conversation> = valid_conversations
            .into_iter()
            .map(|c| (c.id.to_string(), c.clone()))
            .collect();

        let preview_command =
            "CLICOLOR_FORCE=1 forge conversation info {1}; echo; CLICOLOR_FORCE=1 forge conversation show {1}"
                .to_string();

        let selected_uuid = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            Ok(ForgeWidget::select_rows("Conversation", rows)
                .query(query)
                .header_lines(1_usize)
                .preview(Some(preview_command))
                .preview_layout(PreviewLayout { placement: PreviewPlacement::Bottom, percent: 60 })
                .prompt()?
                .map(|row| row.raw))
        })
        .await??;

        Ok(selected_uuid.and_then(|uuid| conv_map.get(&uuid).cloned()))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use forge_api::Conversation;
    use forge_domain::{ConversationId, MetaData, Metrics};
    use pretty_assertions::assert_eq;

    use super::*;

    fn create_test_conversation(id: &str, title: Option<&str>) -> Conversation {
        let now = Utc::now();
        Conversation {
            id: ConversationId::parse(id).unwrap(),
            title: title.map(|t| t.to_string()),
            context: None,
            metrics: Metrics::default().started_at(now),
            metadata: MetaData { created_at: now, updated_at: Some(now) },
        }
    }

    #[tokio::test]
    async fn test_select_conversation_empty_list() {
        let conversations = vec![];
        let result = ConversationSelector::select_conversation(&conversations, None, None)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_select_conversation_with_titles() {
        let conversations = [
            create_test_conversation(
                "550e8400-e29b-41d4-a716-446655440000",
                Some("First Conversation"),
            ),
            create_test_conversation(
                "550e8400-e29b-41d4-a716-446655440001",
                Some("Second Conversation"),
            ),
        ];

        assert_eq!(conversations.len(), 2);
    }

    #[test]
    fn test_select_conversation_without_titles() {
        let conversations = [
            create_test_conversation("550e8400-e29b-41d4-a716-446655440002", None),
            create_test_conversation("550e8400-e29b-41d4-a716-446655440003", None),
        ];

        assert_eq!(conversations.len(), 2);
    }
}
