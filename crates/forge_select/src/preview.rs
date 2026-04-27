use std::collections::BTreeSet;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{cmp, fmt};

use bstr::ByteSlice;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    KeyboardEnhancementFlags, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use crossterm::{execute, queue};
use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config as NucleoConfig, Nucleo, Utf32String};

/// Row rendered by the shared selector UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectRow {
    /// Machine-readable value returned when the row is selected.
    pub raw: String,
    /// User-facing text rendered in the selector list.
    pub display: String,
    /// Additional machine-readable fields used for preview placeholder
    /// expansion.
    pub fields: Vec<String>,
}

impl SelectRow {
    /// Creates a selectable row with a raw value and a display value.
    pub fn new(raw: impl Into<String>, display: impl Into<String>) -> Self {
        let raw = raw.into();
        Self { fields: vec![raw.clone()], raw, display: display.into() }
    }

    /// Creates a non-selectable header row.
    pub fn header(display: impl Into<String>) -> Self {
        Self {
            raw: String::new(),
            display: display.into(),
            fields: Vec::new(),
        }
    }
}

impl fmt::Display for SelectRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display)
    }
}

/// Placement of the selector preview pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewPlacement {
    /// Render preview to the right of the list.
    Right,
    /// Render preview below the list.
    Bottom,
}

/// Preview pane layout configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewLayout {
    /// Preview pane placement.
    pub placement: PreviewPlacement,
    /// Percentage of available space allocated to preview.
    pub percent: u16,
}

impl Default for PreviewLayout {
    fn default() -> Self {
        Self { placement: PreviewPlacement::Right, percent: 50 }
    }
}

struct TerminalGuard {
    raw_mode_was_enabled: bool,
}

impl TerminalGuard {
    fn enter() -> anyhow::Result<Self> {
        let raw_mode_was_enabled = terminal::is_raw_mode_enabled()?;
        enable_raw_mode()?;
        execute!(
            io::stderr(),
            EnterAlternateScreen,
            EnableMouseCapture,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
            Hide
        )?;
        Ok(Self { raw_mode_was_enabled })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            io::stderr(),
            Show,
            PopKeyboardEnhancementFlags,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        if !self.raw_mode_was_enabled {
            let _ = disable_raw_mode();
        }
    }
}

/// Options for running the shared selector UI.
pub struct SelectUiOptions {
    /// Optional prompt text displayed before the query.
    pub prompt: Option<String>,
    /// Optional initial search query.
    pub query: Option<String>,
    /// Rows rendered by the selector.
    pub rows: Vec<SelectRow>,
    /// Number of leading rows treated as non-selectable headers.
    pub header_lines: usize,
    /// Selection mode.
    pub mode: SelectMode,
    /// Optional shell command used to render the selected row preview.
    pub preview: Option<String>,
    /// Preview pane layout.
    pub preview_layout: PreviewLayout,
    /// Optional raw value to focus initially.
    pub initial_raw: Option<String>,
}

/// Runs the shared nucleo-backed selector UI and returns the selected raw
/// value.
///
/// # Errors
///
/// Returns an error if terminal setup, event handling, rendering, or preview
/// command execution setup fails.
pub fn run_select_ui(options: SelectUiOptions) -> anyhow::Result<Option<String>> {
    let SelectUiOptions {
        prompt,
        query,
        rows,
        header_lines,
        mode,
        preview,
        preview_layout,
        initial_raw,
    } = options;
    let header_count = header_lines.min(rows.len());
    let header_rows = rows.iter().take(header_count).collect::<Vec<_>>();
    let data_rows = rows.iter().skip(header_count).cloned().collect::<Vec<_>>();
    if data_rows.is_empty() {
        return Ok(None);
    }

    let mut matcher = Nucleo::new(NucleoConfig::DEFAULT, Arc::new(|| {}), None, 1);
    let injector = matcher.injector();
    for row in data_rows.iter().cloned() {
        injector.push(row, |item, columns| {
            if let Some(column) = columns.get_mut(0) {
                *column = Utf32String::from(item.display.as_str());
            }
        });
    }
    drop(injector);

    let mut query = query.unwrap_or_default();
    matcher
        .pattern
        .reparse(0, &query, CaseMatching::Smart, Normalization::Smart, false);
    let _ = matcher.tick(50);

    let guard = TerminalGuard::enter()?;
    let mut stderr = io::stderr();
    let prompt = prompt.unwrap_or_else(|| "❯ ".to_string());
    let preview_command = preview.unwrap_or_default();
    let mut selected_index = 0usize;
    let mut initial_raw = initial_raw;
    let mut initial_selection_applied = false;
    let mut scroll_offset = 0usize;
    let mut preview_scroll_offset = 0usize;
    let mut queued_indices = BTreeSet::new();
    let mut preview_cache = String::new();
    let mut last_preview_key = String::new();
    let mut last_query = query.clone();
    let mut last_tick = Instant::now();

    loop {
        if query != last_query {
            matcher.pattern.reparse(
                0,
                &query,
                CaseMatching::Smart,
                Normalization::Smart,
                query.starts_with(&last_query),
            );
            let previous_query = last_query.clone();
            last_query = query.clone();
            let _ = matcher.tick(50);
            selected_index = if query.starts_with(&previous_query) {
                selected_index
            } else {
                0
            };
            scroll_offset = 0;
            preview_scroll_offset = 0;
        } else if last_tick.elapsed() >= Duration::from_millis(25) {
            let _ = matcher.tick(10);
            last_tick = Instant::now();
        }

        let matched_rows = matched_rows(&matcher);
        if !initial_selection_applied {
            if let Some(initial_raw) = initial_raw.take()
                && let Some(index) = matched_rows.iter().position(|row| row.raw == initial_raw)
            {
                selected_index = index;
            }
            initial_selection_applied = true;
        }

        if matched_rows.is_empty() {
            selected_index = 0;
            scroll_offset = 0;
        } else if selected_index >= matched_rows.len() {
            selected_index = matched_rows.len().saturating_sub(1);
        }

        let selected_row = matched_rows.get(selected_index).copied();
        let preview_key = selected_row
            .map(|row| format!("{}\0{}", row.raw, query))
            .unwrap_or_default();
        if preview_key != last_preview_key {
            preview_cache = selected_row
                .map(|row| render_preview(&preview_command, row))
                .unwrap_or_else(|| "No matches".to_string());
            preview_scroll_offset = 0;
            last_preview_key = preview_key;
        }

        draw_preview_ui(
            &mut stderr,
            PreviewUi {
                prompt: &prompt,
                query: &query,
                matched_rows: &matched_rows,
                header_rows: &header_rows,
                selected_index,
                scroll_offset: &mut scroll_offset,
                preview: &preview_cache,
                preview_scroll_offset,
                layout: preview_layout,
            },
        )?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    match handle_key_event(
                        key,
                        &mut query,
                        matched_rows.len(),
                        &mut selected_index,
                        !preview_command.is_empty(),
                    ) {
                        PickerAction::Continue => {}
                        PickerAction::PreviewScrollUp => {
                            preview_scroll_offset = preview_scroll_offset.saturating_sub(1);
                        }
                        PickerAction::PreviewScrollDown => {
                            preview_scroll_offset = preview_scroll_offset.saturating_add(1);
                        }
                        PickerAction::PreviewPageUp => {
                            let page_size =
                                preview_content_height(header_rows.len(), preview_layout)
                                    .saturating_sub(1)
                                    .max(1);
                            preview_scroll_offset = preview_scroll_offset.saturating_sub(page_size);
                        }
                        PickerAction::PreviewPageDown => {
                            let page_size =
                                preview_content_height(header_rows.len(), preview_layout)
                                    .saturating_sub(1)
                                    .max(1);
                            preview_scroll_offset = preview_scroll_offset.saturating_add(page_size);
                        }
                        PickerAction::Toggle => {
                            if mode == SelectMode::Multi && selected_row.is_some() {
                                if !queued_indices.remove(&selected_index) {
                                    queued_indices.insert(selected_index);
                                }
                                selected_index = cmp::min(
                                    selected_index + 1,
                                    matched_rows.len().saturating_sub(1),
                                );
                            }
                        }
                        PickerAction::Accept => {
                            if mode == SelectMode::Multi && !queued_indices.is_empty() {
                                drop(guard);
                                for index in &queued_indices {
                                    if let Some(row) = matched_rows.get(*index) {
                                        println!("{}", row.raw);
                                    }
                                }
                                return Ok(None);
                            }

                            if let Some(row) = selected_row {
                                drop(guard);
                                return Ok(Some(row.raw.clone()));
                            }
                        }
                        PickerAction::Exit => {
                            drop(guard);
                            return Ok(None);
                        }
                    }
                }
                Event::Mouse(mouse) => {
                    if !preview_command.is_empty()
                        && mouse_over_preview(
                            mouse.column,
                            mouse.row,
                            header_rows.len(),
                            preview_layout,
                        )
                    {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                preview_scroll_offset = preview_scroll_offset.saturating_sub(3);
                            }
                            MouseEventKind::ScrollDown => {
                                preview_scroll_offset = preview_scroll_offset.saturating_add(3);
                            }
                            _ => {}
                        }
                    } else {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                selected_index = selected_index.saturating_sub(1);
                            }
                            MouseEventKind::ScrollDown => {
                                selected_index = cmp::min(
                                    selected_index.saturating_add(1),
                                    matched_rows.len().saturating_sub(1),
                                );
                            }
                            _ => {}
                        }
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if !preview_command.is_empty() {
            preview_scroll_offset = preview_scroll_offset.min(max_preview_scroll_offset(
                &preview_cache,
                header_rows.len(),
                preview_layout,
            ));
        }
    }
}

/// Selector behavior for accepting one or more rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    /// Accept a single row.
    Single,
    /// Accept multiple rows queued with tab.
    Multi,
}

#[derive(Debug, PartialEq, Eq)]
enum PickerAction {
    Continue,
    Accept,
    Toggle,
    Exit,
    PreviewScrollUp,
    PreviewScrollDown,
    PreviewPageUp,
    PreviewPageDown,
}

fn handle_key_event(
    key: KeyEvent,
    query: &mut String,
    matched_len: usize,
    selected_index: &mut usize,
    has_preview: bool,
) -> PickerAction {
    match key {
        KeyEvent {
            code: KeyCode::Char('c'), modifiers: KeyModifiers::CONTROL, ..
        }
        | KeyEvent { code: KeyCode::Esc, .. } => PickerAction::Exit,
        KeyEvent { code: KeyCode::Char('U'), .. } if has_preview => PickerAction::PreviewPageUp,
        KeyEvent { code: KeyCode::Char('u'), modifiers, .. }
            if has_preview && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            PickerAction::PreviewPageUp
        }
        KeyEvent { code: KeyCode::PageUp, modifiers, .. }
            if has_preview && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            PickerAction::PreviewPageUp
        }
        KeyEvent { code: KeyCode::Char('D'), .. } if has_preview => PickerAction::PreviewPageDown,
        KeyEvent { code: KeyCode::Char('d'), modifiers, .. }
            if has_preview && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            PickerAction::PreviewPageDown
        }
        KeyEvent { code: KeyCode::PageDown, modifiers, .. }
            if has_preview && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            PickerAction::PreviewPageDown
        }
        KeyEvent { code: KeyCode::Char('K'), .. } if has_preview => PickerAction::PreviewScrollUp,
        KeyEvent { code: KeyCode::Char('k'), modifiers, .. }
            if has_preview && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            PickerAction::PreviewScrollUp
        }
        KeyEvent { code: KeyCode::Up, modifiers, .. }
            if has_preview && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            PickerAction::PreviewScrollUp
        }
        KeyEvent { code: KeyCode::Char('J'), .. } if has_preview => PickerAction::PreviewScrollDown,
        KeyEvent { code: KeyCode::Char('j'), modifiers, .. }
            if has_preview && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            PickerAction::PreviewScrollDown
        }
        KeyEvent { code: KeyCode::Down, modifiers, .. }
            if has_preview && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            PickerAction::PreviewScrollDown
        }
        KeyEvent { code: KeyCode::Enter, .. } => PickerAction::Accept,
        KeyEvent { code: KeyCode::BackTab, .. } | KeyEvent { code: KeyCode::Tab, .. } => {
            PickerAction::Toggle
        }
        KeyEvent { code: KeyCode::Up, .. } => {
            if matched_len > 0 {
                *selected_index = selected_index.saturating_sub(1);
            }
            PickerAction::Continue
        }
        KeyEvent { code: KeyCode::Down, .. } => {
            if matched_len > 0 {
                *selected_index = cmp::min(*selected_index + 1, matched_len.saturating_sub(1));
            }
            PickerAction::Continue
        }
        KeyEvent { code: KeyCode::PageUp, .. } => {
            if matched_len > 0 {
                *selected_index = selected_index.saturating_sub(10);
            }
            PickerAction::Continue
        }
        KeyEvent { code: KeyCode::PageDown, .. } => {
            if matched_len > 0 {
                *selected_index = cmp::min(*selected_index + 10, matched_len.saturating_sub(1));
            }
            PickerAction::Continue
        }
        KeyEvent { code: KeyCode::Backspace, .. } => {
            query.pop();
            PickerAction::Continue
        }
        KeyEvent { code: KeyCode::Char(ch), modifiers, .. }
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            query.push(ch);
            PickerAction::Continue
        }
        _ => PickerAction::Continue,
    }
}

fn max_preview_scroll_offset(preview: &str, header_rows: usize, layout: PreviewLayout) -> usize {
    preview
        .lines()
        .count()
        .saturating_sub(preview_content_height(header_rows, layout).max(1))
}

fn preview_content_height(header_rows: usize, layout: PreviewLayout) -> usize {
    let Ok((_, height)) = terminal::size() else {
        return 1;
    };
    let height = ((height.max(6) as u32 * 80) / 100).max(6) as u16;
    let header_height = 2u16.saturating_add(header_rows as u16);
    let body_height = height.saturating_sub(header_height).max(1);

    (match layout.placement {
        PreviewPlacement::Right => body_height,
        PreviewPlacement::Bottom => {
            let preview_height = ((height as u32 * layout.percent as u32) / 100) as u16;
            preview_height
                .clamp(3, body_height.saturating_sub(1).max(3))
                .saturating_sub(2)
        }
    }) as usize
}

fn mouse_over_preview(column: u16, row: u16, header_rows: usize, layout: PreviewLayout) -> bool {
    let Ok((width, height)) = terminal::size() else {
        return false;
    };
    let width = width.max(20);
    let height = ((height.max(6) as u32 * 80) / 100).max(6) as u16;
    let header_height = 2u16.saturating_add(header_rows as u16);
    let body_height = height.saturating_sub(header_height).max(1);

    match layout.placement {
        PreviewPlacement::Right => {
            let preview_width = ((width as u32 * layout.percent as u32) / 100) as u16;
            let preview_width = preview_width.clamp(10, width.saturating_sub(10));
            let list_width = width.saturating_sub(preview_width + 3).max(10);
            let preview_x = list_width + 3;
            column >= preview_x && column < width && row >= header_height && row < height
        }
        PreviewPlacement::Bottom => {
            let preview_height = ((height as u32 * layout.percent as u32) / 100) as u16;
            let preview_height = preview_height.clamp(3, body_height.saturating_sub(1).max(3));
            let list_height = body_height.saturating_sub(preview_height).max(1);
            let preview_y = header_height + list_height;
            column < width && row >= preview_y && row < preview_y.saturating_add(preview_height)
        }
    }
}

fn matched_rows(matcher: &Nucleo<SelectRow>) -> Vec<&SelectRow> {
    matcher
        .snapshot()
        .matched_items(..)
        .map(|item| item.data)
        .collect()
}

fn render_preview(command: &str, row: &SelectRow) -> String {
    if command.trim().is_empty() {
        return String::new();
    }

    let substituted = substitute_preview_command(command, row);
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(&substituted)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match output {
        Ok(output) => {
            let mut rendered = output.stdout.to_str_lossy().into_owned();
            let stderr = output.stderr.to_str_lossy();
            if !stderr.is_empty() {
                if !rendered.is_empty() && !rendered.ends_with('\n') {
                    rendered.push('\n');
                }
                rendered.push_str(&stderr);
            }
            rendered
        }
        Err(error) => format!("Preview command failed: {error}"),
    }
}

fn substitute_preview_command(command: &str, row: &SelectRow) -> String {
    let mut rendered = command.replace("{}", &shell_escape(&row.raw));
    for (index, field) in row.fields.iter().enumerate() {
        let token = format!("{{{}}}", index + 1);
        rendered = rendered.replace(&token, &shell_escape(field));
    }
    rendered
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

struct PreviewUi<'a> {
    prompt: &'a str,
    query: &'a str,
    matched_rows: &'a [&'a SelectRow],
    header_rows: &'a [&'a SelectRow],
    selected_index: usize,
    scroll_offset: &'a mut usize,
    preview: &'a str,
    preview_scroll_offset: usize,
    layout: PreviewLayout,
}

fn draw_preview_ui(stderr: &mut io::Stderr, ui: PreviewUi<'_>) -> anyhow::Result<()> {
    let PreviewUi {
        prompt,
        query,
        matched_rows,
        header_rows,
        selected_index,
        scroll_offset,
        preview,
        preview_scroll_offset,
        layout,
    } = ui;
    let (width, height) = terminal::size()?;
    let width = width.max(20);
    let height = ((height.max(6) as u32 * 80) / 100).max(6) as u16;

    let has_preview = !preview.is_empty();
    let header_height = 2u16.saturating_add(header_rows.len() as u16);
    let body_height = height.saturating_sub(header_height).max(1);

    let (
        list_x,
        list_y,
        list_width,
        list_height,
        preview_x,
        preview_y,
        preview_width,
        preview_height,
    ) = if has_preview {
        match layout.placement {
            PreviewPlacement::Right => {
                let preview_width = ((width as u32 * layout.percent as u32) / 100) as u16;
                let preview_width = preview_width.clamp(10, width.saturating_sub(10));
                let list_width = width.saturating_sub(preview_width + 3).max(10);
                (
                    0,
                    header_height,
                    list_width,
                    body_height,
                    list_width + 3,
                    header_height,
                    preview_width,
                    body_height,
                )
            }
            PreviewPlacement::Bottom => {
                let preview_height = ((height as u32 * layout.percent as u32) / 100) as u16;
                let preview_height = preview_height.clamp(3, body_height.saturating_sub(1).max(3));
                let list_height = body_height.saturating_sub(preview_height).max(1);
                (
                    0,
                    header_height,
                    width,
                    list_height,
                    0,
                    header_height + list_height,
                    width,
                    preview_height,
                )
            }
        }
    } else {
        (0, header_height, width, body_height, 0, height, 0, 0)
    };

    let visible_rows = list_height as usize;
    if visible_rows > 0 {
        if selected_index < *scroll_offset {
            *scroll_offset = selected_index;
        } else if selected_index >= scroll_offset.saturating_add(visible_rows) {
            *scroll_offset = selected_index.saturating_sub(visible_rows.saturating_sub(1));
        }
    }

    queue!(stderr, MoveTo(0, 0), Clear(ClearType::All))?;
    queue!(
        stderr,
        MoveTo(0, 0),
        SetAttribute(Attribute::Bold),
        SetForegroundColor(Color::AnsiValue(110)),
        Print(truncate_line(
            &format!("{}{}", prompt, query),
            width as usize
        )),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    queue!(
        stderr,
        MoveTo(2, 1),
        SetForegroundColor(Color::AnsiValue(144)),
        Print(format!("{}/{}", matched_rows.len(), matched_rows.len())),
        SetForegroundColor(Color::AnsiValue(59)),
        Print(" "),
        Print(truncate_line(
            &"─".repeat(width as usize),
            width.saturating_sub(3 + match_count_width(matched_rows.len())) as usize,
        )),
        ResetColor
    )?;
    for (index, row) in header_rows.iter().enumerate() {
        let row_y = 2u16.saturating_add(index as u16);
        if row_y < header_height {
            queue!(
                stderr,
                MoveTo(2, row_y),
                SetAttribute(Attribute::Bold),
                SetForegroundColor(Color::AnsiValue(109))
            )?;
            queue!(
                stderr,
                Print(truncate_line(
                    &row.display,
                    width.saturating_sub(2) as usize
                ))
            )?;
            queue!(stderr, ResetColor, SetAttribute(Attribute::Reset))?;
        }
    }

    for row_index in 0..list_height {
        queue!(
            stderr,
            MoveTo(list_x, list_y + row_index),
            Clear(ClearType::CurrentLine)
        )?;
        let item_index = *scroll_offset + row_index as usize;
        if let Some(row) = matched_rows.get(item_index) {
            let is_selected = item_index == selected_index;
            let marker = "▌";
            let content_width = list_width.saturating_sub(2) as usize;
            if is_selected {
                queue!(
                    stderr,
                    MoveTo(list_x, list_y + row_index),
                    SetAttribute(Attribute::Bold),
                    SetForegroundColor(Color::AnsiValue(161)),
                    SetBackgroundColor(Color::AnsiValue(236)),
                    Print(marker),
                    SetForegroundColor(Color::AnsiValue(254)),
                    Print(" "),
                    Print(truncate_line(&row.display, content_width)),
                    ResetColor,
                    SetAttribute(Attribute::Reset)
                )?;
            } else {
                queue!(
                    stderr,
                    MoveTo(list_x, list_y + row_index),
                    SetForegroundColor(Color::AnsiValue(236)),
                    Print(marker),
                    ResetColor,
                    Print(" "),
                    Print(truncate_line(&row.display, content_width))
                )?;
            }
        }
    }

    if has_preview {
        match layout.placement {
            PreviewPlacement::Right => {
                let divider_x = list_width + 1;
                for row_index in 0..body_height {
                    queue!(
                        stderr,
                        MoveTo(divider_x, header_height + row_index),
                        Print("│")
                    )?;
                }
            }
            PreviewPlacement::Bottom => {
                queue!(
                    stderr,
                    MoveTo(0, preview_y),
                    SetForegroundColor(Color::AnsiValue(59)),
                    Print("┌"),
                    Print("─".repeat(width.saturating_sub(2) as usize)),
                    Print("┐"),
                    ResetColor
                )?;
            }
        }

        let preview_lines = preview.lines().collect::<Vec<_>>();
        let preview_content_height = match layout.placement {
            PreviewPlacement::Bottom => preview_height.saturating_sub(2),
            PreviewPlacement::Right => preview_height,
        } as usize;
        let preview_scroll_offset = preview_scroll_offset.min(
            preview_lines
                .len()
                .saturating_sub(preview_content_height.max(1)),
        );
        for row_index in 0..preview_height {
            let y = preview_y + row_index;
            if layout.placement == PreviewPlacement::Bottom && row_index == 0 {
                continue;
            }
            if layout.placement == PreviewPlacement::Bottom
                && row_index == preview_height.saturating_sub(1)
            {
                queue!(
                    stderr,
                    MoveTo(preview_x, y),
                    SetForegroundColor(Color::AnsiValue(59)),
                    Print("└"),
                    Print("─".repeat(preview_width.saturating_sub(2) as usize)),
                    Print("┘"),
                    ResetColor
                )?;
                continue;
            }

            let (content_x, content_width) = if layout.placement == PreviewPlacement::Bottom {
                queue!(
                    stderr,
                    MoveTo(preview_x, y),
                    SetForegroundColor(Color::AnsiValue(59)),
                    Print("│"),
                    MoveTo(preview_x + preview_width.saturating_sub(1), y),
                    Print("│"),
                    ResetColor
                )?;
                (preview_x + 2, preview_width.saturating_sub(4))
            } else {
                (preview_x, preview_width)
            };

            queue!(
                stderr,
                MoveTo(content_x, y),
                Print(" ".repeat(content_width as usize))
            )?;
            let line_index = if layout.placement == PreviewPlacement::Bottom {
                preview_scroll_offset + row_index.saturating_sub(1) as usize
            } else {
                preview_scroll_offset + row_index as usize
            };
            if let Some(line) = preview_lines.get(line_index) {
                queue!(
                    stderr,
                    MoveTo(content_x, y),
                    Print(truncate_line(line, content_width as usize))
                )?;
            }

            if layout.placement == PreviewPlacement::Bottom
                && row_index == 1
                && !preview_lines.is_empty()
            {
                let indicator =
                    preview_scroll_indicator(preview_scroll_offset, preview_lines.len());
                let indicator_width = indicator.chars().count() as u16;
                if indicator_width.saturating_add(1) < preview_width {
                    queue!(
                        stderr,
                        MoveTo(
                            preview_x + preview_width.saturating_sub(indicator_width + 2),
                            y
                        ),
                        SetAttribute(Attribute::Reverse),
                        SetForegroundColor(Color::AnsiValue(144)),
                        Print(indicator),
                        ResetColor,
                        SetAttribute(Attribute::Reset),
                        SetForegroundColor(Color::AnsiValue(59)),
                        Print(" "),
                        Print("│"),
                        ResetColor
                    )?;
                }
            }
        }
    }

    stderr.flush()?;
    Ok(())
}

fn preview_scroll_indicator(scroll_offset: usize, line_count: usize) -> String {
    format!("{}/{line_count}", scroll_offset.saturating_add(1))
}

fn match_count_width(count: usize) -> u16 {
    format!("{count}/{count}").chars().count() as u16
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
