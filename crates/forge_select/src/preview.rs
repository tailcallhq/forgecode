use std::collections::BTreeSet;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use std::{cmp, fmt};

use bstr::ByteSlice;
use crossterm::cursor::{Hide, MoveTo, MoveToColumn, MoveUp, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    MouseEventKind,
};
#[cfg(not(windows))]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, Clear, ClearType, disable_raw_mode, enable_raw_mode};
use crossterm::{Command as CrosstermCommand, execute, queue};
use derive_setters::Setters;
use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config as NucleoConfig, Nucleo, Utf32String};

/// Row rendered by the shared selector UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectRow {
    /// Machine-readable value returned when the row is selected.
    pub raw: String,
    /// User-facing text rendered in the selector list.
    pub display: String,
    /// Text indexed by the fuzzy matcher.
    pub search: String,
    /// Additional machine-readable fields used for preview placeholder
    /// expansion.
    pub fields: Vec<String>,
}

impl SelectRow {
    /// Creates a selectable row with a raw value and a display value.
    pub fn new(raw: impl Into<String>, display: impl Into<String>) -> Self {
        let raw = raw.into();
        Self {
            fields: vec![raw.clone()],
            search: raw.clone(),
            raw,
            display: display.into(),
        }
    }

    /// Sets the text indexed by the fuzzy matcher.
    pub fn search(mut self, search: impl Into<String>) -> Self {
        self.search = search.into();
        self
    }

    /// Creates a non-selectable header row.
    pub fn header(display: impl Into<String>) -> Self {
        Self {
            raw: String::new(),
            display: display.into(),
            search: String::new(),
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

const SELECT_VIEWPORT_PERCENT: u16 = 95;

fn max_select_viewport_height(full_height: u16) -> u16 {
    let full_height = full_height.max(1);
    ((full_height as u32 * SELECT_VIEWPORT_PERCENT as u32) / 100)
        .max(1)
        .min(full_height as u32) as u16
}

fn select_viewport_height(full_height: u16, desired_height: u16) -> u16 {
    let full_height = full_height.max(1);
    let desired_height = desired_height.max(1);
    if desired_height <= full_height {
        desired_height
    } else {
        max_select_viewport_height(full_height)
    }
}

fn reserve_inline_viewport_space(
    stderr: &mut io::Stderr,
    desired_height: u16,
) -> io::Result<(u16, u16)> {
    let (_, full_height) = terminal::size()?;
    let reserved_height = select_viewport_height(full_height, desired_height);

    for _ in 0..reserved_height {
        queue!(stderr, Print("\r\n"))?;
    }
    queue!(stderr, MoveUp(reserved_height), MoveToColumn(0))?;
    stderr.flush()?;

    let viewport_top_row = full_height.saturating_sub(reserved_height.max(1));
    Ok((reserved_height, viewport_top_row))
}

fn desired_select_viewport_height(
    header_rows: usize,
    matched_rows: usize,
    preview_lines: usize,
    layout: PreviewLayout,
) -> u16 {
    let header_height = 2u16.saturating_add(header_rows as u16);
    let list_height = (matched_rows as u16).max(1);
    let preview_lines = preview_lines as u16;

    match layout.placement {
        PreviewPlacement::Right => header_height.saturating_add(list_height),
        PreviewPlacement::Bottom if preview_lines > 0 => header_height
            .saturating_add(list_height)
            .saturating_add(preview_lines.saturating_add(2)),
        PreviewPlacement::Bottom => header_height.saturating_add(list_height),
    }
    .max(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DeleteLines(u16);

impl CrosstermCommand for DeleteLines {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        if self.0 > 0 {
            write!(f, "\u{1b}[{}M", self.0)?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewportMoveTo {
    x: u16,
    y: u16,
    top_row: u16,
}

impl CrosstermCommand for ViewportMoveTo {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        MoveTo(self.x, self.top_row.saturating_add(self.y)).write_ansi(f)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }
}

fn viewport_move_to(x: u16, y: u16, top_row: u16) -> ViewportMoveTo {
    ViewportMoveTo { x, y, top_row }
}

fn clear_rendered_viewport(
    stderr: &mut io::Stderr,
    reserved_height: u16,
    viewport_top_row: u16,
) -> io::Result<()> {
    let (_, full_height) = terminal::size()?;
    let max_top_row = full_height.saturating_sub(reserved_height.max(1));
    let viewport_top_row = viewport_top_row.min(max_top_row);

    for row_index in 0..reserved_height {
        queue!(
            stderr,
            viewport_move_to(0, row_index, viewport_top_row),
            Clear(ClearType::CurrentLine)
        )?;
    }
    queue!(
        stderr,
        MoveTo(0, viewport_top_row),
        DeleteLines(reserved_height)
    )?;
    stderr.flush()
}

struct TerminalGuard {
    raw_mode_was_enabled: bool,
    keyboard_enhancement_enabled: bool,
}

impl TerminalGuard {
    fn enter() -> anyhow::Result<Self> {
        let raw_mode_was_enabled = terminal::is_raw_mode_enabled()?;
        enable_raw_mode()?;
        execute!(io::stderr(), EnableMouseCapture, Hide)?;
        let keyboard_enhancement_enabled = enable_keyboard_enhancement()?;
        Ok(Self { raw_mode_was_enabled, keyboard_enhancement_enabled })
    }
}

#[cfg(not(windows))]
fn enable_keyboard_enhancement() -> anyhow::Result<bool> {
    execute!(
        io::stderr(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;
    Ok(true)
}

#[cfg(windows)]
fn enable_keyboard_enhancement() -> anyhow::Result<bool> {
    Ok(false)
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stderr(), Show);
        if self.keyboard_enhancement_enabled {
            let _ = execute!(io::stderr(), PopKeyboardEnhancementFlags);
        }
        let _ = execute!(io::stderr(), DisableMouseCapture);
        if !self.raw_mode_was_enabled {
            let _ = disable_raw_mode();
        }
    }
}

/// Options for running the shared selector UI.
#[derive(Debug, Setters)]
#[setters(into)]
pub struct SelectUiOptions {
    /// Optional prompt text displayed before the query.
    #[setters(skip)]
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

impl SelectUiOptions {
    /// Creates selector options for the provided prompt and rows.
    pub fn new(prompt: impl Into<String>, rows: Vec<SelectRow>) -> Self {
        Self {
            prompt: Some(prompt.into()),
            query: None,
            rows,
            header_lines: 0,
            mode: SelectMode::Single,
            preview: None,
            preview_layout: PreviewLayout::default(),
            initial_raw: None,
        }
    }

    /// Runs the selector and returns the selected row.
    ///
    /// # Errors
    ///
    /// Returns an error if terminal setup, event handling, rendering, or
    /// preview command execution setup fails.
    pub fn prompt(self) -> anyhow::Result<Option<SelectRow>> {
        let rows = self.rows.clone();
        let selected_raw = run_select_ui(self)?;
        Ok(selected_raw.and_then(|raw| rows.into_iter().find(|row| row.raw == raw)))
    }

    /// Runs the selector and returns all selected rows.
    ///
    /// # Errors
    ///
    /// Returns an error if terminal setup, event handling, rendering, or
    /// preview command execution setup fails.
    pub fn prompt_multi(self) -> anyhow::Result<Option<Vec<SelectRow>>> {
        let rows = self.rows.clone();
        let selected_raws = run_select_ui_values(self)?;
        Ok(selected_raws.map(|raws| {
            raws.into_iter()
                .filter_map(|raw| rows.iter().find(|row| row.raw == raw).cloned())
                .collect()
        }))
    }
}

/// Runs the shared nucleo-backed selector UI and returns the selected raw
/// value.
///
/// # Errors
///
/// Returns an error if terminal setup, event handling, rendering, or preview
/// command execution setup fails.
pub fn run_select_ui(options: SelectUiOptions) -> anyhow::Result<Option<String>> {
    Ok(run_select_ui_values(options)?.and_then(|values| values.into_iter().next()))
}

fn run_select_ui_values(options: SelectUiOptions) -> anyhow::Result<Option<Vec<String>>> {
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
                *column = Utf32String::from(item.search.as_str());
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
    let initial_matched_rows = matched_rows(&matcher);
    // When a preview command is present, reserve the maximum available viewport
    // height upfront. Without this, the initial reservation (calculated with
    // zero preview lines) is too small: once a preview renders it consumes the
    // configured percentage of the reserved space and leaves only 1–2 rows for
    // the list, even when many items match.
    let initial_desired_height = if !preview_command.is_empty() {
        u16::MAX
    } else {
        desired_select_viewport_height(
            header_rows.len(),
            initial_matched_rows.len(),
            0,
            preview_layout,
        )
    };
    let (reserved_height, viewport_top_row) =
        reserve_inline_viewport_space(&mut stderr, initial_desired_height)?;
    let mut selected_index = 0usize;
    let mut initial_raw = initial_raw;
    let mut initial_selection_applied = false;
    let mut scroll_offset = 0usize;
    let mut preview_scroll_offset = 0usize;
    let mut queued_indices = BTreeSet::new();
    let mut preview_cache = String::new();
    let mut last_preview_key = String::new();
    let mut last_query = query.clone();

    let mut needs_render = true;
    loop {
        if query != last_query {
            matcher.pattern.reparse(
                0,
                &query,
                CaseMatching::Smart,
                Normalization::Smart,
                query.starts_with(&last_query),
            );
            last_query = query.clone();
            let _ = matcher.tick(50);
            selected_index = 0;
            scroll_offset = 0;
            preview_scroll_offset = 0;
            needs_render = true;
        }

        let matched_rows = matched_rows(&matcher);
        if !initial_selection_applied {
            if let Some(initial_raw) = initial_raw.take()
                && let Some(index) = matched_rows.iter().position(|row| row.raw == initial_raw)
            {
                selected_index = index;
                needs_render = true;
            }
            initial_selection_applied = true;
        }

        if matched_rows.is_empty() {
            if selected_index != 0 || scroll_offset != 0 {
                needs_render = true;
            }
            selected_index = 0;
            scroll_offset = 0;
        } else if selected_index >= matched_rows.len() {
            selected_index = matched_rows.len().saturating_sub(1);
            needs_render = true;
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
            needs_render = true;
        }

        let rendered_preview = if preview_command.is_empty() {
            ""
        } else {
            &preview_cache
        };

        if needs_render {
            draw_preview_ui(
                &mut stderr,
                PreviewUi {
                    prompt: &prompt,
                    query: &query,
                    total_rows: data_rows.len(),
                    matched_rows: &matched_rows,
                    header_rows: &header_rows,
                    selected_index,
                    scroll_offset: &mut scroll_offset,
                    preview: rendered_preview,
                    preview_scroll_offset,
                    layout: preview_layout,
                    reserved_height,
                    viewport_top_row,
                },
            )?;
            needs_render = false;
        }

        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(key) => {
                    match handle_key_event(
                        key,
                        &mut query,
                        matched_rows.len(),
                        &mut selected_index,
                        !preview_command.is_empty(),
                    ) {
                        PickerAction::Continue => {
                            needs_render = true;
                        }
                        PickerAction::PreviewScrollUp => {
                            preview_scroll_offset = preview_scroll_offset.saturating_sub(1);
                            needs_render = true;
                        }
                        PickerAction::PreviewScrollDown => {
                            preview_scroll_offset = preview_scroll_offset.saturating_add(1);
                            needs_render = true;
                        }
                        PickerAction::PreviewPageUp => {
                            let page_size = preview_content_height(
                                header_rows.len(),
                                matched_rows.len(),
                                &preview_cache,
                                preview_layout,
                                reserved_height,
                            )
                            .saturating_sub(1)
                            .max(1);
                            preview_scroll_offset = preview_scroll_offset.saturating_sub(page_size);
                            needs_render = true;
                        }
                        PickerAction::PreviewPageDown => {
                            let page_size = preview_content_height(
                                header_rows.len(),
                                matched_rows.len(),
                                &preview_cache,
                                preview_layout,
                                reserved_height,
                            )
                            .saturating_sub(1)
                            .max(1);
                            preview_scroll_offset = preview_scroll_offset.saturating_add(page_size);
                            needs_render = true;
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
                                needs_render = true;
                            }
                        }
                        PickerAction::Accept => {
                            if mode == SelectMode::Multi && !queued_indices.is_empty() {
                                clear_rendered_viewport(
                                    &mut stderr,
                                    reserved_height,
                                    viewport_top_row,
                                )?;
                                drop(guard);
                                let selected = queued_indices
                                    .iter()
                                    .filter_map(|index| matched_rows.get(*index))
                                    .map(|row| row.raw.clone())
                                    .collect::<Vec<_>>();
                                return Ok(Some(selected));
                            }

                            if let Some(row) = selected_row {
                                clear_rendered_viewport(
                                    &mut stderr,
                                    reserved_height,
                                    viewport_top_row,
                                )?;
                                drop(guard);
                                return Ok(Some(vec![row.raw.clone()]));
                            }
                        }
                        PickerAction::Exit => {
                            clear_rendered_viewport(
                                &mut stderr,
                                reserved_height,
                                viewport_top_row,
                            )?;
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
                            matched_rows.len(),
                            &preview_cache,
                            preview_layout,
                            reserved_height,
                        )
                    {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                preview_scroll_offset = preview_scroll_offset.saturating_sub(3);
                                needs_render = true;
                            }
                            MouseEventKind::ScrollDown => {
                                preview_scroll_offset = preview_scroll_offset.saturating_add(3);
                                needs_render = true;
                            }
                            _ => {}
                        }
                    } else {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                selected_index = selected_index.saturating_sub(1);
                                needs_render = true;
                            }
                            MouseEventKind::ScrollDown => {
                                selected_index = cmp::min(
                                    selected_index.saturating_add(1),
                                    matched_rows.len().saturating_sub(1),
                                );
                                needs_render = true;
                            }
                            _ => {}
                        }
                    }
                }
                Event::Resize(_, _) => {
                    needs_render = true;
                }
                _ => {}
            }
        }

        if !preview_command.is_empty() {
            let clamped_offset = preview_scroll_offset.min(max_preview_scroll_offset(
                &preview_cache,
                header_rows.len(),
                matched_rows.len(),
                preview_layout,
                reserved_height,
            ));
            if clamped_offset != preview_scroll_offset {
                preview_scroll_offset = clamped_offset;
                needs_render = true;
            }
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

fn max_preview_scroll_offset(
    preview: &str,
    header_rows: usize,
    matched_rows: usize,
    layout: PreviewLayout,
    reserved_height: u16,
) -> usize {
    preview.lines().count().saturating_sub(
        preview_content_height(header_rows, matched_rows, preview, layout, reserved_height).max(1),
    )
}

fn preview_content_height(
    header_rows: usize,
    matched_rows: usize,
    preview: &str,
    layout: PreviewLayout,
    reserved_height: u16,
) -> usize {
    let Ok((_, height)) = terminal::size() else {
        return 1;
    };
    let desired_height =
        desired_select_viewport_height(header_rows, matched_rows, preview.lines().count(), layout);
    let height = select_viewport_height(height, desired_height).min(reserved_height);
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

fn mouse_over_preview(
    column: u16,
    row: u16,
    header_rows: usize,
    matched_rows: usize,
    preview: &str,
    layout: PreviewLayout,
    reserved_height: u16,
) -> bool {
    let Ok((width, height)) = terminal::size() else {
        return false;
    };
    let width = width.max(20);
    let desired_height =
        desired_select_viewport_height(header_rows, matched_rows, preview.lines().count(), layout);
    let height = select_viewport_height(height, desired_height).min(reserved_height);
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
    total_rows: usize,
    matched_rows: &'a [&'a SelectRow],
    header_rows: &'a [&'a SelectRow],
    selected_index: usize,
    scroll_offset: &'a mut usize,
    preview: &'a str,
    preview_scroll_offset: usize,
    layout: PreviewLayout,
    reserved_height: u16,
    viewport_top_row: u16,
}

fn draw_preview_ui(stderr: &mut io::Stderr, ui: PreviewUi<'_>) -> anyhow::Result<()> {
    let PreviewUi {
        prompt,
        query,
        total_rows,
        matched_rows,
        header_rows,
        selected_index,
        scroll_offset,
        preview,
        preview_scroll_offset,
        layout,
        reserved_height,
        viewport_top_row,
    } = ui;
    let (width, full_height) = terminal::size()?;
    let width = width.max(20);

    let has_preview = !preview.is_empty();
    // Always render into the full reserved region. Computing a smaller
    // desired_height from current content and capping height to it would leave
    // the already-reserved terminal rows blank, wasting visible space.
    // Keep one safety row at the bottom of the reserved region to avoid
    // terminal-specific implicit wrap/scroll behavior when writing at the
    // final visible row.
    let height = reserved_height.saturating_sub(1).max(1);
    let max_top_row = full_height.saturating_sub(height.max(1));
    let top_offset = viewport_top_row.min(max_top_row);
    let header_height = 3u16.saturating_add(header_rows.len() as u16);
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

    for row_index in 0..reserved_height {
        queue!(
            stderr,
            viewport_move_to(0, row_index, top_offset),
            Clear(ClearType::CurrentLine)
        )?;
    }
    queue!(
        stderr,
        viewport_move_to(0, 0, top_offset),
        SetAttribute(Attribute::Bold),
        SetForegroundColor(Color::AnsiValue(110)),
        Print(truncate_line(
            &format_prompt_query(prompt, query),
            width as usize
        )),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    queue!(
        stderr,
        viewport_move_to(2, 1, top_offset),
        SetForegroundColor(Color::AnsiValue(144)),
        Print(format!("{}/{}", matched_rows.len(), total_rows)),
        SetForegroundColor(Color::AnsiValue(59)),
        Print(" "),
        Print(truncate_line(
            &"─".repeat(width as usize),
            width.saturating_sub(3 + match_count_width(matched_rows.len(), total_rows)) as usize,
        )),
        ResetColor
    )?;
    for (index, row) in header_rows.iter().enumerate() {
        let row_y = 2u16.saturating_add(index as u16);
        if row_y < header_height {
            queue!(
                stderr,
                viewport_move_to(2, row_y, top_offset),
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
            viewport_move_to(list_x, list_y + row_index, top_offset),
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
                    viewport_move_to(list_x, list_y + row_index, top_offset),
                    SetAttribute(Attribute::Bold),
                    SetForegroundColor(Color::AnsiValue(161)),
                    SetBackgroundColor(Color::AnsiValue(236)),
                    Print(marker),
                    SetForegroundColor(Color::AnsiValue(254)),
                    Print(" "),
                    Print(truncate_line_with_ellipsis(&row.display, content_width)),
                    ResetColor,
                    SetAttribute(Attribute::Reset)
                )?;
            } else {
                queue!(
                    stderr,
                    viewport_move_to(list_x, list_y + row_index, top_offset),
                    SetForegroundColor(Color::AnsiValue(236)),
                    Print(marker),
                    ResetColor,
                    Print(" "),
                    Print(truncate_line_with_ellipsis(&row.display, content_width))
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
                        viewport_move_to(divider_x, header_height + row_index, top_offset),
                        Print("│")
                    )?;
                }
            }
            PreviewPlacement::Bottom => {
                queue!(
                    stderr,
                    viewport_move_to(0, preview_y, top_offset),
                    SetForegroundColor(Color::AnsiValue(59)),
                    Print("┌"),
                    Print("─".repeat(width.saturating_sub(2) as usize)),
                    Print("┐"),
                    ResetColor
                )?;
            }
        }

        let preview_content_height = match layout.placement {
            PreviewPlacement::Bottom => preview_height.saturating_sub(2),
            PreviewPlacement::Right => preview_height,
        } as usize;
        let preview_width_for_content = match layout.placement {
            PreviewPlacement::Bottom => preview_width.saturating_sub(4),
            PreviewPlacement::Right => preview_width,
        } as usize;
        let preview_lines = wrap_preview_lines(preview, preview_width_for_content.max(1));
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
                    viewport_move_to(preview_x, y, top_offset),
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
                    viewport_move_to(preview_x, y, top_offset),
                    SetForegroundColor(Color::AnsiValue(59)),
                    Print("│"),
                    viewport_move_to(preview_x + preview_width.saturating_sub(1), y, top_offset),
                    Print("│"),
                    ResetColor
                )?;
                (preview_x + 2, preview_width.saturating_sub(4))
            } else {
                (preview_x, preview_width)
            };

            queue!(
                stderr,
                viewport_move_to(content_x, y, top_offset),
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
                    viewport_move_to(content_x, y, top_offset),
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
                        viewport_move_to(
                            preview_x + preview_width.saturating_sub(indicator_width + 2),
                            y,
                            top_offset,
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

fn wrap_preview_lines(preview: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return Vec::new();
    }

    preview
        .lines()
        .flat_map(|line| wrap_ansi_line(line, max_width))
        .collect()
}

fn wrap_ansi_line(line: &str, max_width: usize) -> Vec<String> {
    const WRAP_ICON: &str = "↪ ";
    const WRAP_ICON_WIDTH: usize = 2;

    if line.is_empty() {
        return vec![String::new()];
    }

    let mut wrapped_lines = Vec::new();
    let mut current_line = String::new();
    let mut visible_width = 0usize;
    let mut chars = line.chars().peekable();
    let mut is_continuation = false;

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            current_line.push(ch);
            for ansi_ch in chars.by_ref() {
                current_line.push(ansi_ch);
                if ansi_ch.is_ascii_alphabetic() || ansi_ch == '~' {
                    break;
                }
            }
            continue;
        }

        let current_limit = if is_continuation {
            max_width.saturating_sub(WRAP_ICON_WIDTH).max(1)
        } else {
            max_width
        };

        if visible_width >= current_limit {
            let pushed = if is_continuation {
                format!("{WRAP_ICON}{current_line}")
            } else {
                current_line.clone()
            };
            wrapped_lines.push(pushed);
            current_line = String::new();
            visible_width = 0;
            is_continuation = true;
        }

        current_line.push(ch);
        visible_width = visible_width.saturating_add(1);
    }

    if !current_line.is_empty() {
        let pushed = if is_continuation {
            format!("{WRAP_ICON}{current_line}")
        } else {
            current_line
        };
        wrapped_lines.push(pushed);
    }

    if wrapped_lines.is_empty() {
        vec![String::new()]
    } else {
        wrapped_lines
    }
}

fn format_prompt_query(prompt: &str, query: &str) -> String {
    if query.is_empty() || prompt.ends_with(char::is_whitespace) {
        format!("{prompt}{query}")
    } else {
        format!("{prompt} {query}")
    }
}

fn match_count_width(matched: usize, total: usize) -> u16 {
    format!("{matched}/{total}").chars().count() as u16
}

fn truncate_line_with_ellipsis(value: &str, max_width: usize) -> String {
    const ELLIPSIS: &str = "…";
    let full_width = value.chars().count();
    if full_width <= max_width {
        return value.to_string();
    }

    if max_width <= ELLIPSIS.len() {
        return ELLIPSIS.chars().take(max_width).collect();
    }

    let keep_width = max_width.saturating_sub(ELLIPSIS.len());
    let prefix: String = value.chars().take(keep_width).collect();
    format!("{prefix}{ELLIPSIS}")
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
    fn test_desired_select_viewport_height_right_ignores_preview_line_count() {
        let fixture = PreviewLayout { placement: PreviewPlacement::Right, percent: 50 };
        let actual = desired_select_viewport_height(1, 2, 285, fixture);
        let expected = 5;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_desired_select_viewport_height_bottom_includes_preview_line_count() {
        let fixture = PreviewLayout { placement: PreviewPlacement::Bottom, percent: 50 };
        let actual = desired_select_viewport_height(1, 2, 4, fixture);
        let expected = 11;
        assert_eq!(actual, expected);
    }
}
