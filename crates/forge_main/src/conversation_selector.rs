use anyhow::Result;
use chrono::Utc;
use forge_api::Conversation;
use forge_domain::ConversationId;
use forge_select::{ForgeWidget, PreviewLayout, PreviewPlacement, SelectRow};

use crate::display_constants::markers;
use crate::info::Info;
use crate::porcelain::Porcelain;

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

        // Build Info structure for display
        let now = Utc::now();
        let mut info = Info::new();

        for conv in &valid_conversations {
            let title = conv
                .title
                .as_deref()
                .map(|t| t.to_string())
                .unwrap_or_else(|| markers::EMPTY.to_string());

            let duration = now.signed_duration_since(
                conv.metadata.updated_at.unwrap_or(conv.metadata.created_at),
            );
            let duration =
                std::time::Duration::from_secs((duration.num_minutes() * 60).max(0) as u64);
            let time_ago = if duration.is_zero() {
                "now".to_string()
            } else {
                format!("{} ago", humantime::format_duration(duration))
            };

            info = info
                .add_title(conv.id)
                .add_key_value("Title", title)
                .add_key_value("Updated", time_ago);
        }

        // Convert to porcelain, drop the UUID title column (col 0), truncate the
        // Title column for display, uppercase headers
        let porcelain_output = Porcelain::from(&info)
            .drop_col(0)
            .truncate(0, 60)
            .uppercase_headers();
        let porcelain_str = porcelain_output.to_string();

        let all_lines: Vec<&str> = porcelain_str.lines().collect();
        if all_lines.is_empty() {
            return Ok(None);
        }

        // Build SelectRow items for the shared Rust selector UI.
        // Each row stores the UUID in `fields[0]` so that `{1}` in the preview
        // command resolves to the conversation ID. The `raw` field is what gets
        // returned on selection (the UUID).
        let mut rows: Vec<SelectRow> = Vec::with_capacity(all_lines.len());

        // Header row (non-selectable via header_lines=1)
        if let Some(header) = all_lines.first() {
            rows.push(SelectRow::header(header.to_string()));
        }

        // Data rows: each maps to a conversation
        for (i, line) in all_lines.iter().skip(1).enumerate() {
            if let Some(conv) = valid_conversations.get(i) {
                let uuid = conv.id.to_string();
                rows.push(SelectRow {
                    raw: uuid.clone(),
                    display: line.to_string(),
                    search: line.to_string(),
                    fields: vec![uuid],
                });
            }
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
