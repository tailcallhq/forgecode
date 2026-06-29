use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use forge_api::Conversation;
use forge_domain::{ConversationId, ConversationSort};
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
        let title = self.conv.title.as_deref().unwrap_or(markers::EMPTY);

        // Truncate title to fixed width (50 chars) with ellipsis if longer
        let max_title_width = 50;
        let title_display = if title.len() > max_title_width {
            format!("{}…", &title[..max_title_width])
        } else {
            title.to_string()
        };

        // Pad title to fixed width for alignment
        let title_padded = format!("{:<width$}", title_display, width = max_title_width + 1);

        let duration = self.now.signed_duration_since(
            self.conv
                .metadata
                .updated_at
                .unwrap_or(self.conv.metadata.created_at),
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

        // Subagent breadcrumb: show ↳ prefix when conversation has a parent
        let breadcrumb = if self.conv.parent_id.is_some() {
            "↳ "
        } else {
            ""
        };

        // Fixed-column date alignment (right-aligned in 10-char field)
        write!(f, "{}{} {:>10}", breadcrumb, title_padded, time_ago)
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
    /// The `query` parameter filters/searches conversations if provided (enables FTS).
    /// The `sort` parameter controls the display order (updated, created, turns, title, cwd).
    ///
    /// Returns the selected conversation, or None if the user cancelled.
    pub async fn select_conversation(
        conversations: &[Conversation],
        _current_conversation_id: Option<ConversationId>,
        query: Option<String>,
        sort: ConversationSort,
    ) -> Result<Option<Conversation>> {
        if conversations.is_empty() {
            return Ok(None);
        }

        // Build the list of conversations to display, optionally filtered by query
        let mut final_conversations: Vec<&Conversation> = conversations
            .iter()
            .filter(|c| c.context.is_some())
            .filter(|c| {
                // Apply query filter if provided
                if let Some(ref q) = query {
                    let q_lower = q.to_lowercase();
                    c.title
                        .as_ref()
                        .map(|t| t.to_lowercase().contains(&q_lower))
                        .unwrap_or(false)
                } else {
                    true
                }
            })
            .collect();

        if final_conversations.is_empty() {
            return Ok(None);
        }

        // Apply sorting based on the current sort order
        final_conversations.sort_by(|a, b| {
            match sort {
                ConversationSort::Updated => {
                    // Most recent first (DESC)
                    let a_time = a.metadata.updated_at.unwrap_or(a.metadata.created_at);
                    let b_time = b.metadata.updated_at.unwrap_or(b.metadata.created_at);
                    b_time.cmp(&a_time)
                }
                ConversationSort::Created => {
                    // Newest first (DESC)
                    b.metadata.created_at.cmp(&a.metadata.created_at)
                }
                ConversationSort::Turns => {
                    // By message count (DESC), then by updated_at (DESC)
                    match (b.message_count, a.message_count) {
                        (Some(b_count), Some(a_count)) => {
                            let count_cmp = b_count.cmp(&a_count);
                            if count_cmp != std::cmp::Ordering::Equal {
                                count_cmp
                            } else {
                                let a_time = a.metadata.updated_at.unwrap_or(a.metadata.created_at);
                                let b_time = b.metadata.updated_at.unwrap_or(b.metadata.created_at);
                                b_time.cmp(&a_time)
                            }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => {
                            let a_time = a.metadata.updated_at.unwrap_or(a.metadata.created_at);
                            let b_time = b.metadata.updated_at.unwrap_or(b.metadata.created_at);
                            b_time.cmp(&a_time)
                        }
                    }
                }
                ConversationSort::Title => {
                    // Alphabetical ASC, nulls last
                    match (&a.title, &b.title) {
                        (Some(a_title), Some(b_title)) => a_title.cmp(b_title),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                }
                ConversationSort::Cwd => {
                    // By cwd ASC, nulls last; then by updated_at DESC
                    match (&a.cwd, &b.cwd) {
                        (Some(a_cwd), Some(b_cwd)) => {
                            let cwd_cmp = a_cwd.cmp(b_cwd);
                            if cwd_cmp != std::cmp::Ordering::Equal {
                                cwd_cmp
                            } else {
                                let a_time = a.metadata.updated_at.unwrap_or(a.metadata.created_at);
                                let b_time = b.metadata.updated_at.unwrap_or(b.metadata.created_at);
                                b_time.cmp(&a_time)
                            }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => {
                            let a_time = a.metadata.updated_at.unwrap_or(a.metadata.created_at);
                            let b_time = b.metadata.updated_at.unwrap_or(b.metadata.created_at);
                            b_time.cmp(&a_time)
                        }
                    }
                }
            }
        });

        // Build SelectRow items directly — no Info/Porcelain overhead.
        // This keeps the selector fast even with thousands of conversations.
        let now = Utc::now();
        let mut rows: Vec<SelectRow> = Vec::with_capacity(final_conversations.len() + 1);
        rows.push(SelectRow::header(
            "Title                                          Updated   ",
        ));

        for conv in &final_conversations {
            let uuid = conv.id.to_string();
            let display = FastConversationRow::new(conv, now).to_string();
            rows.push(SelectRow {
                raw: uuid.clone(),
                display: display.clone(),
                search: display,
                fields: vec![uuid],
            });
        }

        // Build a lookup map from UUID to Arc<Conversation> for the result.
        // Using Arc avoids cloning every Conversation twice (once for the row
        // raw UUID and once for the lookup map) — big win on 6k+ lists.
        let conv_map: HashMap<String, Arc<Conversation>> = final_conversations
            .iter()
            .map(|c| (c.id.to_string(), Arc::new((*c).clone())))
            .collect::<HashMap<_, _>>();

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

        Ok(selected_uuid.and_then(|uuid| conv_map.get(&uuid).map(|c| c.as_ref().clone())))
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
            cwd: None,
            message_count: None,
            parent_id: None,
            source: None,
        }
    }

    #[tokio::test]
    async fn test_select_conversation_empty_list() {
        let conversations = vec![];
        let result = ConversationSelector::select_conversation(
            &conversations,
            None,
            None,
            ConversationSort::Updated,
        )
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
