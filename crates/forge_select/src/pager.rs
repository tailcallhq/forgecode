use std::cmp;
use std::io::{self, Write};
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    MouseEventKind,
};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, disable_raw_mode, enable_raw_mode};
use crossterm::{execute, queue};

/// Result of the permission pager interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPagerResult {
    /// User accepted the operation.
    Accept,
    /// User accepted and wants to remember this choice (create a policy rule).
    AcceptAndRemember,
    /// User rejected the operation.
    Reject,
}

/// Runs an interactive permission pager that displays content and lets the
/// user Accept, AcceptAndRemember, or Reject.
///
/// The pager renders the given panel content in a scrollable view and always
/// shows a footer bar with keybindings at the bottom of the terminal.
///
/// # Arguments
/// * `panel` - The formatted panel text to display (e.g. from
///   `PermissionCase::format_panel()`)
///
/// # Returns
/// * `PermissionPagerResult::Accept` — Enter pressed
/// * `PermissionPagerResult::AcceptAndRemember` — 'A' pressed
/// * `PermissionPagerResult::Reject` — 'R', Esc, or Ctrl+C pressed
///
/// # Errors
/// Returns an error if terminal setup, event handling, or rendering fails.
pub fn show_permission_pager(panel: &str) -> anyhow::Result<PermissionPagerResult> {
    let mut stderr = io::BufWriter::new(io::stderr());

    // Enter raw mode + hide cursor + enable mouse
    let raw_mode_was_enabled = terminal::is_raw_mode_enabled()?;
    enable_raw_mode()?;
    execute!(stderr, EnableMouseCapture, Hide)?;

    let result = run_pager(&mut stderr, panel);

    // Restore terminal state
    let _ = execute!(stderr, Show, DisableMouseCapture);
    let _ = stderr.flush();
    if !raw_mode_was_enabled {
        let _ = disable_raw_mode();
    }

    result
}

fn run_pager(
    stderr: &mut impl Write,
    panel: &str,
) -> anyhow::Result<PermissionPagerResult> {
    let lines: Vec<&str> = panel.lines().collect();
    let total_lines = lines.len();
    let mut scroll_offset = 0usize;
    let mut dirty = true;
    let mut content_height = 0usize;

    loop {
        if dirty {
            let (width, height) = terminal::size()?;
            let footer_height = 2u16; // 2 rows for footer: separator bar + keybindings
            content_height = height.saturating_sub(footer_height).max(1) as usize;

            // Clamp scroll offset
            if total_lines > content_height {
                let max_offset = total_lines - content_height;
                if scroll_offset > max_offset {
                    scroll_offset = max_offset;
                }
            } else {
                scroll_offset = 0;
                content_height = total_lines;
            }

            // Clear entire screen first to wipe any leftover content
            // (status messages, tool output, etc.) that was written before the
            // pager entered raw mode. Without this, old content persists and
            // overlaps with the pager.
            queue!(stderr, Clear(ClearType::All))?;

            // Clear content area only (not footer)
            for row in 0..content_height {
                queue!(
                    stderr,
                    crossterm::cursor::MoveTo(0, row as u16),
                    Clear(ClearType::CurrentLine)
                )?;
            }

            // Draw content lines
            let visible_end = cmp::min(scroll_offset + content_height, total_lines);
            for (i, line_idx) in (scroll_offset..visible_end).enumerate() {
                let line = lines[line_idx];
                queue!(
                    stderr,
                    crossterm::cursor::MoveTo(0, i as u16),
                    Print(truncate_line(line, width as usize))
                )?;
            }

            // Scroll indicator in top-right (if scrolled)
            if total_lines > content_height {
                let indicator =
                    format!("{}/{}", scroll_offset.saturating_add(1), total_lines);
                if indicator.len() + 2 < width as usize {
                    queue!(
                        stderr,
                        crossterm::cursor::MoveTo(
                            width.saturating_sub(indicator.len() as u16 + 2),
                            0,
                        ),
                        SetForegroundColor(Color::DarkYellow),
                        Print(&indicator),
                        ResetColor
                    )?;
                }
            }

            // Clear and redraw footer each time
            let footer_y = height.saturating_sub(footer_height);
            for row in footer_y..height {
                queue!(
                    stderr,
                    crossterm::cursor::MoveTo(0, row),
                    Clear(ClearType::CurrentLine)
                )?;
            }
            // Separator line
            let separator = "─".repeat(width as usize);
            queue!(
                stderr,
                crossterm::cursor::MoveTo(0, footer_y),
                SetForegroundColor(Color::DarkGrey),
                Print(separator),
                ResetColor
            )?;
            // Keybindings bar
            let keybindings = format!(
                " [Enter] Accept  [A] Accept & Remember  [R] Reject  ↑↓u/d PgUp/PgDn Scroll"
            );
            queue!(
                stderr,
                crossterm::cursor::MoveTo(0, footer_y + 1),
                SetForegroundColor(Color::Cyan),
                Print(truncate_line(&keybindings, width as usize)),
                ResetColor
            )?;

            stderr.flush()?;
            dirty = false;
        }

        // Wait for event
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(key) => {
                    let old_offset = scroll_offset;
                    match handle_pager_key(
                        key,
                        total_lines,
                        content_height,
                        &mut scroll_offset,
                    ) {
                        PagerAction::Accept => return Ok(PermissionPagerResult::Accept),
                        PagerAction::AcceptAndRemember => {
                            return Ok(PermissionPagerResult::AcceptAndRemember)
                        }
                        PagerAction::Reject => return Ok(PermissionPagerResult::Reject),
                        PagerAction::Continue => {
                            if scroll_offset != old_offset {
                                dirty = true;
                            }
                        }
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        if total_lines > content_height {
                            let old = scroll_offset;
                            scroll_offset = scroll_offset.saturating_sub(3);
                            dirty = scroll_offset != old;
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if total_lines > content_height {
                            let old = scroll_offset;
                            scroll_offset = cmp::min(
                                scroll_offset.saturating_add(3),
                                total_lines.saturating_sub(content_height),
                            );
                            dirty = scroll_offset != old;
                        }
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {
                    dirty = true;
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PagerAction {
    Accept,
    AcceptAndRemember,
    Reject,
    Continue,
}

fn handle_pager_key(
    key: KeyEvent,
    total_lines: usize,
    content_height: usize,
    scroll_offset: &mut usize,
) -> PagerAction {
    match key {
        // Accept
        KeyEvent { code: KeyCode::Enter, .. } => PagerAction::Accept,
        // Accept and Remember
        KeyEvent { code: KeyCode::Char('a'), modifiers, .. }
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            PagerAction::AcceptAndRemember
        }
        KeyEvent { code: KeyCode::Char('A'), .. } => PagerAction::AcceptAndRemember,
        // Reject
        KeyEvent { code: KeyCode::Char('r'), modifiers, .. }
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            PagerAction::Reject
        }
        KeyEvent { code: KeyCode::Char('R'), .. } => PagerAction::Reject,
        KeyEvent { code: KeyCode::Esc, .. } => PagerAction::Reject,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => PagerAction::Reject,
        // Scroll up (also 'u' for vi-style)
        KeyEvent { code: KeyCode::Up, .. } => {
            *scroll_offset = scroll_offset.saturating_sub(1);
            PagerAction::Continue
        }
        KeyEvent { code: KeyCode::Char('u'), .. } => {
            let page = content_height.saturating_sub(1).max(1);
            *scroll_offset = scroll_offset.saturating_sub(page);
            PagerAction::Continue
        }
        KeyEvent { code: KeyCode::PageUp, .. } => {
            let page = content_height.saturating_sub(1).max(1);
            *scroll_offset = scroll_offset.saturating_sub(page);
            PagerAction::Continue
        }
        // Scroll down (also 'd' for vi-style)
        KeyEvent { code: KeyCode::Down, .. } => {
            let max_offset = total_lines.saturating_sub(content_height);
            *scroll_offset = cmp::min(scroll_offset.saturating_add(1), max_offset);
            PagerAction::Continue
        }
        KeyEvent { code: KeyCode::Char('d'), .. } => {
            let page = content_height.saturating_sub(1).max(1);
            let max_offset = total_lines.saturating_sub(content_height);
            *scroll_offset = cmp::min(scroll_offset.saturating_add(page), max_offset);
            PagerAction::Continue
        }
        KeyEvent { code: KeyCode::PageDown, .. } => {
            let page = content_height.saturating_sub(1).max(1);
            let max_offset = total_lines.saturating_sub(content_height);
            *scroll_offset = cmp::min(scroll_offset.saturating_add(page), max_offset);
            PagerAction::Continue
        }
        _ => PagerAction::Continue,
    }
}

fn truncate_line(value: &str, max_width: usize) -> String {
    let mut rendered = String::new();
    let mut visible_width = 0usize;
    let mut chars = value.chars().peekable();
    let mut truncated = false;
    let mut has_ansi = false;

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            has_ansi = true;
            rendered.push(ch);
            for ansi_ch in chars.by_ref() {
                rendered.push(ansi_ch);
                if ansi_ch.is_ascii_alphabetic() || ansi_ch == '~' {
                    break;
                }
            }
            continue;
        }

        if visible_width >= max_width {
            truncated = true;
            break;
        }

        rendered.push(ch);
        visible_width = visible_width.saturating_add(1);
    }

    if truncated && has_ansi {
        rendered.push_str("\u{1b}[0m");
    }

    rendered
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_truncate_line_simple() {
        let fixture = "Hello, World!";
        let actual = truncate_line(fixture, 5);
        let expected = "Hello";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_truncate_line_no_truncation() {
        let fixture = "Hi";
        let actual = truncate_line(fixture, 80);
        let expected = "Hi";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_truncate_line_ansi() {
        let fixture = "\u{1b}[31mRed\u{1b}[0m text";
        let actual = truncate_line(fixture, 10);
        // Should keep ANSI sequences
        assert!(actual.starts_with("\u{1b}[31m"));
        assert!(actual.len() <= 30);
    }

    #[test]
    fn test_handle_pager_key_accept() {
        let mut offset = 0usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::Accept);
    }

    #[test]
    fn test_handle_pager_key_accept_and_remember() {
        let mut offset = 0usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::AcceptAndRemember);
    }

    #[test]
    fn test_handle_pager_key_accept_and_remember_uppercase() {
        let mut offset = 0usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::AcceptAndRemember);
    }

    #[test]
    fn test_handle_pager_key_reject() {
        let mut offset = 0usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::Reject);
    }

    #[test]
    fn test_handle_pager_key_reject_escape() {
        let mut offset = 0usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::Reject);
    }

    #[test]
    fn test_handle_pager_key_scroll_up() {
        let mut offset = 10usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::Continue);
        assert_eq!(offset, 9);
    }

    #[test]
    fn test_handle_pager_key_scroll_down() {
        let mut offset = 10usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::Continue);
        assert_eq!(offset, 11);
    }

    #[test]
    fn test_handle_pager_key_scroll_down_clamped() {
        let mut offset = 90usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::Continue);
        assert_eq!(offset, 80);
    }

    #[test]
    fn test_handle_pager_key_page_down() {
        let mut offset = 10usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::Continue);
        assert_eq!(offset, 29);
    }

    #[test]
    fn test_handle_pager_key_page_up() {
        let mut offset = 50usize;
        let result = handle_pager_key(
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
            100,
            20,
            &mut offset,
        );
        assert_eq!(result, PagerAction::Continue);
        assert_eq!(offset, 31);
    }
}
