use forge_domain::{Todo, TodoStatus};

/// Controls the styling applied to a rendered todo line.
enum TodoLineStyle {
    /// Bold styling used for new or changed todos.
    Bold,
    /// Dim styling used for unchanged todos.
    Dim,
}

/// Renders one todo line with icon and ANSI styling.
///
/// # Arguments
///
/// * `todo` - Todo item to render.
/// * `line_style` - Emphasis style for the line.
fn format_todo_line(todo: &Todo, line_style: TodoLineStyle) -> String {
    use console::style;

    let checkbox = match todo.status {
        TodoStatus::Completed => "󰄵",
        TodoStatus::InProgress => "󰄗",
        TodoStatus::Pending => "󰄱",
        TodoStatus::Cancelled => "󰅙",
    };

    let content = match todo.status {
        TodoStatus::Completed | TodoStatus::Cancelled => {
            style(todo.content.as_str()).strikethrough().to_string()
        }
        _ => todo.content.clone(),
    };

    let line = format!("  {checkbox} {content}");
    let styled = match (&todo.status, line_style) {
        (TodoStatus::Pending, TodoLineStyle::Bold) => style(line).white().bold().to_string(),
        (TodoStatus::Pending, TodoLineStyle::Dim) => style(line).white().dim().to_string(),
        (TodoStatus::Cancelled, TodoLineStyle::Bold) => style(line).red().bold().to_string(),
        (TodoStatus::Cancelled, TodoLineStyle::Dim) => style(line).red().dim().to_string(),
        (TodoStatus::InProgress, TodoLineStyle::Bold) => style(line).cyan().bold().to_string(),
        (TodoStatus::InProgress, TodoLineStyle::Dim) => style(line).cyan().dim().to_string(),
        (TodoStatus::Completed, TodoLineStyle::Bold) => style(line).green().bold().to_string(),
        (TodoStatus::Completed, TodoLineStyle::Dim) => style(line).green().dim().to_string(),
    };

    format!("{styled}\n")
}

/// Formats a todo diff showing all todos in `after` plus removed todos from
/// `before`.
///
/// # Arguments
///
/// * `before` - Previous todo list state.
/// * `after` - New todo list state.
pub(crate) fn format_todos_diff(before: &[Todo], after: &[Todo]) -> String {
    use console::style;

    let before_map: std::collections::HashMap<&str, &Todo> =
        before.iter().map(|todo| (todo.id.as_str(), todo)).collect();
    let after_map: std::collections::HashMap<&str, &Todo> =
        after.iter().map(|todo| (todo.id.as_str(), todo)).collect();

    let mut result = "\n".to_string();

    // Walk `before` in insertion order: emit the current version of surviving
    // items, or the removed rendering for items that were dropped.
    for before_todo in before {
        if let Some(after_todo) = after_map.get(before_todo.id.as_str()).copied() {
            // Item still exists — render with bold/dim based on whether it changed.
            let is_changed = before_todo.status != after_todo.status
                || before_todo.content != after_todo.content;
            let line_style = if is_changed {
                TodoLineStyle::Bold
            } else {
                TodoLineStyle::Dim
            };
            result.push_str(&format_todo_line(after_todo, line_style));
        } else {
            // Item was removed — render with status-aware styling.
            let content = style(before_todo.content.as_str())
                .strikethrough()
                .to_string();
            if before_todo.status == TodoStatus::Completed {
                // Removed completed: dimmed white checkmark (historical done)
                result.push_str(&format!(
                    "  {}\n",
                    style(format!("󰄵 {content}")).white().dim()
                ));
            } else {
                // Removed non-completed: use the correct status icon in red
                let checkbox = match before_todo.status {
                    TodoStatus::InProgress => "󰄗",
                    TodoStatus::Pending => "󰄱",
                    TodoStatus::Cancelled => "󰅙",
                    TodoStatus::Completed => "󰄵",
                };
                result.push_str(&format!(
                    "  {}\n",
                    style(format!("{checkbox} {content}")).red()
                ));
            }
        }
    }

    // Append newly-added items (present in `after` but not in `before`) in
    // their original insertion order.
    for todo in after {
        if !before_map.contains_key(todo.id.as_str()) {
            result.push_str(&format_todo_line(todo, TodoLineStyle::Bold));
        }
    }

    result
}

/// Formats todos as ANSI-styled checklist lines.
///
/// # Arguments
///
/// * `todos` - Todo list to format.
pub(crate) fn format_todos(todos: &[Todo]) -> String {
    if todos.is_empty() {
        return String::new();
    }

    let mut result = "\n".to_string();

    for todo in todos {
        result.push_str(&format_todo_line(todo, TodoLineStyle::Dim));
    }

    result
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use console::{
        colors_enabled, colors_enabled_stderr, set_colors_enabled, set_colors_enabled_stderr,
        strip_ansi_codes,
    };
    use forge_domain::{ChatResponseContent, Environment, Todo, TodoStatus};
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;

    use crate::fmt::content::FormatContent;
    use crate::operation::ToolOperation;

    static ANSI_STYLE_LOCK: Mutex<()> = Mutex::new(());

    struct ColorStateGuard {
        stdout: bool,
        stderr: bool,
    }

    impl ColorStateGuard {
        fn force_enabled() -> Self {
            let stdout = colors_enabled();
            let stderr = colors_enabled_stderr();
            set_colors_enabled(true);
            set_colors_enabled_stderr(true);
            Self { stdout, stderr }
        }
    }

    impl Drop for ColorStateGuard {
        fn drop(&mut self) {
            set_colors_enabled(self.stdout);
            set_colors_enabled_stderr(self.stderr);
        }
    }

    fn fixture_environment() -> Environment {
        use fake::{Fake, Faker};
        Faker.fake()
    }

    fn fixture_todo(content: &str, id: &str, status: TodoStatus) -> Todo {
        Todo::new(content).id(id).status(status)
    }

    fn fixture_todo_write_output_raw(before: Vec<Todo>, after: Vec<Todo>) -> String {
        let _lock = ANSI_STYLE_LOCK
            .lock()
            .expect("ANSI style lock should not be poisoned");
        let _colors = ColorStateGuard::force_enabled();
        let setup = ToolOperation::TodoWrite { before, after };
        let actual = setup.to_content(&fixture_environment());

        if let Some(ChatResponseContent::ToolOutput(output)) = actual {
            output
        } else {
            panic!("Expected ToolOutput content")
        }
    }

    #[test]
    fn test_todo_write_removed_in_progress_renders_with_in_progress_icon_in_raw_snapshot() {
        // before: Write migrations is in_progress
        // after:  empty (it was cancelled/removed)
        let setup = (
            vec![fixture_todo(
                "Write migrations",
                "1",
                TodoStatus::InProgress,
            )],
            Vec::new(),
        );

        // Verify icon (strip color) — must be 󰄗, NOT 󰄱
        let plain = fixture_todo_write_output(setup.0.clone(), setup.1.clone());
        let expected_plain = "\n  󰄗 Write migrations\n";
        assert_eq!(plain, expected_plain);

        let raw = fixture_todo_write_output_raw(setup.0, setup.1);
        assert_snapshot!(raw);
    }

    #[test]
    fn test_todo_write_removed_pending_renders_with_pending_icon_in_raw_snapshot() {
        let setup = (
            vec![fixture_todo("Pending task", "1", TodoStatus::Pending)],
            Vec::new(),
        );

        let plain = fixture_todo_write_output(setup.0.clone(), setup.1.clone());
        let expected_plain = "\n  󰄱 Pending task\n";
        assert_eq!(plain, expected_plain);

        let raw = fixture_todo_write_output_raw(setup.0, setup.1);
        assert_snapshot!(raw);
    }

    fn fixture_todo_write_output(before: Vec<Todo>, after: Vec<Todo>) -> String {
        let setup = ToolOperation::TodoWrite { before, after };
        let actual = setup.to_content(&fixture_environment());

        if let Some(ChatResponseContent::ToolOutput(output)) = actual {
            strip_ansi_codes(output.as_str()).to_string()
        } else {
            panic!("Expected ToolOutput content")
        }
    }

    #[test]
    fn test_todo_write_mixed_changes_snapshot() {
        let setup = (
            vec![
                fixture_todo("Task 1", "1", TodoStatus::Pending),
                fixture_todo("Task 2", "2", TodoStatus::InProgress),
            ],
            vec![
                fixture_todo("Task 1", "1", TodoStatus::Completed),
                fixture_todo("Task 3", "3", TodoStatus::Pending),
            ],
        );

        let actual = fixture_todo_write_output(setup.0, setup.1);
        assert_snapshot!(actual);
    }

    #[test]
    fn test_todo_write_removed_completed_todos_render_as_dimmed_done() {
        let setup = (
            vec![fixture_todo("Done", "1", TodoStatus::Completed)],
            Vec::new(),
        );

        let actual = fixture_todo_write_output(setup.0, setup.1);
        let expected = "\n  󰄵 Done\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_todos_preserves_insertion_order() {
        // Items are given in insertion order: Second was added first, First second.
        // Output must reflect that insertion order, not alphabetical or id-sorted.
        let setup = vec![
            fixture_todo("Second", "2", TodoStatus::Pending),
            fixture_todo("First", "1", TodoStatus::Pending),
        ];

        let actual = strip_ansi_codes(super::format_todos(&setup).as_str()).to_string();
        let expected = "\n  󰄱 Second\n  󰄱 First\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_todo_write_dump_flow_in_same_order() {
        let step_1 = vec![
            fixture_todo(
                "Generate JSONL input file with all 59 cases",
                "1",
                TodoStatus::InProgress,
            ),
            fixture_todo(
                "Create JSON schema file for structured output",
                "2",
                TodoStatus::Pending,
            ),
            fixture_todo("Create system prompt template", "3", TodoStatus::Pending),
            fixture_todo("Create user prompt template", "4", TodoStatus::Pending),
            fixture_todo("Test with 2-3 cases first", "5", TodoStatus::Pending),
            fixture_todo("Run for all cases", "6", TodoStatus::Pending),
        ];
        let step_2 = vec![
            fixture_todo(
                "Generate JSONL input file with all 59 cases",
                "1",
                TodoStatus::Completed,
            ),
            fixture_todo(
                "Create JSON schema file for structured output",
                "2",
                TodoStatus::InProgress,
            ),
            fixture_todo("Create system prompt template", "3", TodoStatus::Pending),
            fixture_todo("Create user prompt template", "4", TodoStatus::Pending),
            fixture_todo("Test with 2-3 cases first", "5", TodoStatus::Pending),
            fixture_todo("Run for all cases", "6", TodoStatus::Pending),
        ];
        let step_3 = vec![
            fixture_todo(
                "Create JSON schema file for structured output",
                "2",
                TodoStatus::Completed,
            ),
            fixture_todo("Create system prompt template", "3", TodoStatus::InProgress),
            fixture_todo("Create user prompt template", "4", TodoStatus::Pending),
            fixture_todo("Test with 2-3 cases first", "5", TodoStatus::Pending),
            fixture_todo("Run for all cases", "6", TodoStatus::Pending),
        ];
        let step_4 = vec![
            fixture_todo("Create system prompt template", "3", TodoStatus::Completed),
            fixture_todo("Create user prompt template", "4", TodoStatus::Completed),
            fixture_todo("Test with 2-3 cases first", "5", TodoStatus::InProgress),
            fixture_todo("Run for all cases", "6", TodoStatus::Pending),
        ];

        let actual_1 = fixture_todo_write_output(Vec::new(), step_1.clone());
        let expected_1 = "\n  󰄗 Generate JSONL input file with all 59 cases\n  󰄱 Create JSON schema file for structured output\n  󰄱 Create system prompt template\n  󰄱 Create user prompt template\n  󰄱 Test with 2-3 cases first\n  󰄱 Run for all cases\n";
        assert_eq!(actual_1, expected_1);

        let actual_2 = fixture_todo_write_output(step_1.clone(), step_2.clone());
        let expected_2 = "\n  󰄵 Generate JSONL input file with all 59 cases\n  󰄗 Create JSON schema file for structured output\n  󰄱 Create system prompt template\n  󰄱 Create user prompt template\n  󰄱 Test with 2-3 cases first\n  󰄱 Run for all cases\n";
        assert_eq!(actual_2, expected_2);

        let actual_3 = fixture_todo_write_output(step_2.clone(), step_3.clone());
        let expected_3 = "\n  󰄵 Generate JSONL input file with all 59 cases\n  󰄵 Create JSON schema file for structured output\n  󰄗 Create system prompt template\n  󰄱 Create user prompt template\n  󰄱 Test with 2-3 cases first\n  󰄱 Run for all cases\n";
        assert_eq!(actual_3, expected_3);

        let actual_4 = fixture_todo_write_output(step_3, step_4);
        let expected_4 = "\n  󰄵 Create JSON schema file for structured output\n  󰄵 Create system prompt template\n  󰄵 Create user prompt template\n  󰄗 Test with 2-3 cases first\n  󰄱 Run for all cases\n";
        assert_eq!(actual_4, expected_4);
    }
}
