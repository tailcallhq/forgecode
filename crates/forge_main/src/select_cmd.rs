use std::cmp;
use std::io::{self, BufRead, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use crossterm::{execute, queue};
use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config as NucleoConfig, Nucleo, Utf32String};
use nucleo_picker::error::PickError;
use nucleo_picker::{PickerOptions, render::StrRenderer};
use regex::Regex;

use crate::cli::SelectArgs;

/// Run the interactive fuzzy picker.
///
/// Reads items from stdin, presents them in a nucleo-based TUI, and prints the
/// selected item(s) to stdout. When stdin is not a terminal, `/dev/tty` is
/// opened for keyboard input.
pub fn run_select(args: SelectArgs) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut items = Vec::new();

    for line in stdin.lock().lines() {
        items.push(line?);
    }

    if items.is_empty() {
        std::process::exit(1);
    }

    #[cfg(unix)]
    if !stdin.is_terminal() {
        redirect_stdin_to_tty()?;
    }

    if args.preview.is_some() {
        return run_select_with_preview(args, items);
    }

    let mut picker_opts = PickerOptions::default()
        .reversed(true)
        .case_matching(nucleo_picker::CaseMatching::Smart);

    if let Some(query) = args.query {
        picker_opts = picker_opts.query(query);
    }

    let mut picker: nucleo_picker::Picker<String, _> = picker_opts.picker(StrRenderer);
    picker.extend_exact(items);

    if args.multi {
        match picker.pick_multi() {
            Ok(selection) if selection.is_empty() => std::process::exit(1),
            Ok(selection) => {
                for item in selection.iter() {
                    println!("{}", item);
                }
            }
            Err(PickError::NotInteractive) | Err(PickError::UserInterrupted) => {
                std::process::exit(1);
            }
            Err(error) => anyhow::bail!("Picker error: {error}"),
        }
    } else {
        match picker.pick() {
            Ok(Some(item)) => println!("{}", item),
            Ok(None) => std::process::exit(1),
            Err(PickError::NotInteractive) | Err(PickError::UserInterrupted) => {
                std::process::exit(1);
            }
            Err(error) => anyhow::bail!("Picker error: {error}"),
        }
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectRow {
    raw: String,
    display: String,
    fields: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FieldSelector {
    Index(usize),
    RangeFrom(usize),
    RangeInclusive(usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FieldPart {
    value: String,
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviewPlacement {
    Right,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PreviewLayout {
    placement: PreviewPlacement,
    percent: u16,
}

impl Default for PreviewLayout {
    fn default() -> Self {
        Self {
            placement: PreviewPlacement::Right,
            percent: 50,
        }
    }
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stderr(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stderr(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn run_select_with_preview(args: SelectArgs, items: Vec<String>) -> anyhow::Result<()> {
    if args.multi {
        anyhow::bail!("Preview mode does not support --multi yet");
    }

    let rows = build_rows(&items, args.delimiter.as_deref(), args.with_nth.as_deref())?;
    if rows.is_empty() {
        std::process::exit(1);
    }

    let mut matcher = Nucleo::new(NucleoConfig::DEFAULT, Arc::new(|| {}), None, 1);
    let injector = matcher.injector();
    for row in rows.iter().cloned() {
        injector.push(row, |item, columns| {
            if let Some(column) = columns.get_mut(0) {
                *column = Utf32String::from(item.display.as_str());
            }
        });
    }
    drop(injector);

    let mut query = args.query.unwrap_or_default();
    matcher
        .pattern
        .reparse(0, &query, CaseMatching::Smart, Normalization::Smart, false);
    let _ = matcher.tick(50);

    let guard = TerminalGuard::enter()?;
    let mut stderr = io::stderr();
    let prompt = args.prompt.unwrap_or_else(|| "❯ ".to_string());
    let preview_command = args.preview.unwrap_or_default();
    let preview_layout = parse_preview_window(args.preview_window.as_deref());
    let mut selected_index = 0usize;
    let mut scroll_offset = 0usize;
    let mut preview_cache = String::new();
    let mut last_preview_key = String::new();
    let mut last_query = query.clone();
    let mut last_tick = Instant::now();

    if rows.len() > 1 {
        selected_index = 1;
    }

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
            selected_index = if query.starts_with(&previous_query) { selected_index } else { 0 };
            scroll_offset = 0;
        } else if last_tick.elapsed() >= Duration::from_millis(25) {
            let _ = matcher.tick(10);
            last_tick = Instant::now();
        }

        let matched_rows = matched_rows(&matcher);
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
            last_preview_key = preview_key;
        }

        draw_preview_ui(
            &mut stderr,
            PreviewUi {
                prompt: &prompt,
                query: &query,
                matched_rows: &matched_rows,
                selected_index,
                scroll_offset: &mut scroll_offset,
                preview: &preview_cache,
                layout: preview_layout,
            },
        )?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => match handle_key_event(key, &mut query, matched_rows.len(), &mut selected_index) {
                    PickerAction::Continue => {}
                    PickerAction::Accept => {
                        if let Some(row) = selected_row {
                            drop(guard);
                            println!("{}", row.raw);
                            return Ok(());
                        }
                    }
                    PickerAction::Exit => {
                        drop(guard);
                        std::process::exit(1);
                    }
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PickerAction {
    Continue,
    Accept,
    Exit,
}

fn handle_key_event(
    key: KeyEvent,
    query: &mut String,
    matched_len: usize,
    selected_index: &mut usize,
) -> PickerAction {
    match key {
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
        | KeyEvent {
            code: KeyCode::Esc, ..
        } => PickerAction::Exit,
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => PickerAction::Accept,
        KeyEvent {
            code: KeyCode::Up, ..
        }
        | KeyEvent {
            code: KeyCode::BackTab,
            ..
        } => {
            if matched_len > 0 {
                *selected_index = selected_index.saturating_sub(1);
            }
            PickerAction::Continue
        }
        KeyEvent {
            code: KeyCode::Down,
            ..
        }
        | KeyEvent {
            code: KeyCode::Tab, ..
        } => {
            if matched_len > 0 {
                *selected_index = cmp::min(*selected_index + 1, matched_len.saturating_sub(1));
            }
            PickerAction::Continue
        }
        KeyEvent {
            code: KeyCode::PageUp,
            ..
        } => {
            if matched_len > 0 {
                *selected_index = selected_index.saturating_sub(10);
            }
            PickerAction::Continue
        }
        KeyEvent {
            code: KeyCode::PageDown,
            ..
        } => {
            if matched_len > 0 {
                *selected_index = cmp::min(*selected_index + 10, matched_len.saturating_sub(1));
            }
            PickerAction::Continue
        }
        KeyEvent {
            code: KeyCode::Backspace,
            ..
        } => {
            query.pop();
            PickerAction::Continue
        }
        KeyEvent {
            code: KeyCode::Char(ch),
            modifiers,
            ..
        } if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
            query.push(ch);
            PickerAction::Continue
        }
        _ => PickerAction::Continue,
    }
}

fn build_rows(
    items: &[String],
    delimiter: Option<&str>,
    with_nth: Option<&str>,
) -> anyhow::Result<Vec<SelectRow>> {
    let delimiter_regex = match delimiter {
        Some(value) => Some(Regex::new(value).with_context(|| format!("Invalid delimiter regex: {value}"))?),
        None => None,
    };
    let display_fields = parse_with_nth(with_nth)?;

    let row_parts = items
        .iter()
        .map(|item| split_field_parts(item, delimiter_regex.as_ref()))
        .collect::<Vec<_>>();

    let column_widths = compute_display_widths(&row_parts, display_fields.as_deref());

    let rows = items
        .iter()
        .zip(row_parts)
        .map(|(item, parts)| SelectRow {
            raw: item.clone(),
            display: build_display(item, &parts, display_fields.as_deref(), &column_widths),
            fields: parts.into_iter().map(|part| part.value).collect(),
        })
        .collect();

    Ok(rows)
}

fn parse_with_nth(with_nth: Option<&str>) -> anyhow::Result<Option<Vec<FieldSelector>>> {
    let Some(value) = with_nth else {
        return Ok(None);
    };

    let mut selectors = Vec::new();
    for part in value.split(',').map(str::trim).filter(|part| !part.is_empty()) {
        if let Some((start, end)) = part.split_once("..") {
            let start = start
                .trim()
                .parse::<usize>()
                .with_context(|| format!("Invalid --with-nth field: {part}"))?;

            if end.trim().is_empty() {
                selectors.push(FieldSelector::RangeFrom(start));
            } else {
                let end = end
                    .trim()
                    .parse::<usize>()
                    .with_context(|| format!("Invalid --with-nth field: {part}"))?;
                selectors.push(FieldSelector::RangeInclusive(start, end));
            }
        } else {
            selectors.push(FieldSelector::Index(
                part.parse::<usize>()
                    .with_context(|| format!("Invalid --with-nth field: {part}"))?,
            ));
        }
    }

    if selectors.is_empty() {
        return Ok(None);
    }

    Ok(Some(selectors))
}

fn split_field_parts(item: &str, delimiter: Option<&Regex>) -> Vec<FieldPart> {
    match delimiter {
        Some(regex) => {
            let mut parts = Vec::new();
            let mut last_end = 0usize;

            for delimiter_match in regex.find_iter(item) {
                let field = item
                    .get(last_end..delimiter_match.start())
                    .unwrap_or_default()
                    .trim();
                if !field.is_empty() {
                    parts.push(FieldPart {
                        value: field.to_string(),
                        start: last_end,
                        end: delimiter_match.start(),
                    });
                }
                last_end = delimiter_match.end();
            }

            let field = item.get(last_end..).unwrap_or_default().trim();
            if !field.is_empty() {
                parts.push(FieldPart {
                    value: field.to_string(),
                    start: last_end,
                    end: item.len(),
                });
            }

            if parts.is_empty() {
                vec![FieldPart {
                    value: item.to_string(),
                    start: 0,
                    end: item.len(),
                }]
            } else {
                parts
            }
        }
        None => vec![FieldPart {
            value: item.to_string(),
            start: 0,
            end: item.len(),
        }],
    }
}

fn build_display(
    item: &str,
    parts: &[FieldPart],
    display_fields: Option<&[FieldSelector]>,
    column_widths: &[usize],
) -> String {
    let Some(display_fields) = display_fields else {
        return item.to_string();
    };

    let selected_parts = select_display_parts(parts, display_fields);

    if selected_parts.is_empty() {
        return item.to_string();
    }

    selected_parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            if index == selected_parts.len() - 1 {
                part.value.clone()
            } else {
                format!(
                    "{:<width$}",
                    part.value,
                    width = column_widths.get(index).copied().unwrap_or(part.value.chars().count())
                )
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

fn compute_display_widths(
    rows: &[Vec<FieldPart>],
    display_fields: Option<&[FieldSelector]>,
) -> Vec<usize> {
    let Some(display_fields) = display_fields else {
        return Vec::new();
    };

    let mut widths = Vec::new();
    for row in rows {
        for (index, part) in select_display_parts(row, display_fields).iter().enumerate() {
            let width = part.end.saturating_sub(part.start);
            if widths.len() <= index {
                widths.push(width);
            } else if let Some(current_width) = widths.get_mut(index)
                && *current_width < width
            {
                *current_width = width;
            }
        }
    }

    widths
}

fn select_display_parts(parts: &[FieldPart], selectors: &[FieldSelector]) -> Vec<FieldPart> {
    let mut selected = Vec::new();

    for selector in selectors {
        match *selector {
            FieldSelector::Index(index) => {
                if let Some(value) = parts.get(index.saturating_sub(1)) {
                    selected.push(value.clone());
                }
            }
            FieldSelector::RangeFrom(start) => {
                for value in parts.iter().skip(start.saturating_sub(1)) {
                    selected.push(value.clone());
                }
            }
            FieldSelector::RangeInclusive(start, end) => {
                for value in parts
                    .iter()
                    .skip(start.saturating_sub(1))
                    .take(end.saturating_sub(start).saturating_add(1))
                {
                    selected.push(value.clone());
                }
            }
        }
    }

    selected
}

fn parse_preview_window(value: Option<&str>) -> PreviewLayout {
    let Some(value) = value else {
        return PreviewLayout::default();
    };

    let placement = if value.contains("down") || value.contains("bottom") || value.contains("up") {
        PreviewPlacement::Bottom
    } else {
        PreviewPlacement::Right
    };

    let percent = value
        .split(':')
        .flat_map(|segment| segment.split(','))
        .find_map(|segment| segment.trim().strip_suffix('%'))
        .and_then(|segment| segment.parse::<u16>().ok())
        .map(|percent| percent.clamp(10, 90))
        .unwrap_or(50);

    PreviewLayout { placement, percent }
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
            let mut rendered = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr);
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
    selected_index: usize,
    scroll_offset: &'a mut usize,
    preview: &'a str,
    layout: PreviewLayout,
}

fn draw_preview_ui(stderr: &mut io::Stderr, ui: PreviewUi<'_>) -> anyhow::Result<()> {
    let PreviewUi {
        prompt,
        query,
        matched_rows,
        selected_index,
        scroll_offset,
        preview,
        layout,
    } = ui;
    let (width, height) = terminal::size()?;
    let width = width.max(20);
    let height = height.max(6);

    let header_height = 1u16;
    let status_height = 1u16;
    let body_height = height.saturating_sub(header_height + status_height).max(1);

    let (list_x, list_y, list_width, list_height, preview_x, preview_y, preview_width, preview_height) =
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
                let preview_height = ((body_height as u32 * layout.percent as u32) / 100) as u16;
                let preview_height = preview_height.clamp(3, body_height.saturating_sub(2).max(3));
                let list_height = body_height.saturating_sub(preview_height + 1).max(1);
                (0, header_height, width, list_height, 0, header_height + list_height + 1, width, preview_height)
            }
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
    queue!(stderr, MoveTo(0, 0), Print(truncate_line(&format!("{}{}", prompt, query), width as usize)))?;

    for row_index in 0..list_height {
        queue!(stderr, MoveTo(list_x, list_y + row_index), Clear(ClearType::CurrentLine))?;
        let item_index = *scroll_offset + row_index as usize;
        if let Some(row) = matched_rows.get(item_index) {
            let prefix = if item_index == selected_index { "> " } else { "  " };
            let content_width = list_width.saturating_sub(prefix.chars().count() as u16) as usize;
            let line = format!("{prefix}{}", truncate_line(&row.display, content_width));
            if item_index == selected_index {
                queue!(stderr, SetAttribute(Attribute::Reverse))?;
            }
            queue!(stderr, MoveTo(list_x, list_y + row_index), Print(line))?;
            if item_index == selected_index {
                queue!(stderr, SetAttribute(Attribute::Reset))?;
            }
        }
    }

    match layout.placement {
        PreviewPlacement::Right => {
            let divider_x = list_width + 1;
            for row_index in 0..body_height {
                queue!(stderr, MoveTo(divider_x, header_height + row_index), Print("│"))?;
            }
        }
        PreviewPlacement::Bottom => {
            queue!(stderr, MoveTo(0, preview_y.saturating_sub(1)), Clear(ClearType::CurrentLine))?;
            queue!(stderr, MoveTo(0, preview_y.saturating_sub(1)), Print("─".repeat(width as usize)))?;
        }
    }

    let preview_lines = preview.lines().collect::<Vec<_>>();
    for row_index in 0..preview_height {
        queue!(stderr, MoveTo(preview_x, preview_y + row_index), Print(" ".repeat(preview_width as usize)))?;
        if let Some(line) = preview_lines.get(row_index as usize) {
            queue!(stderr, MoveTo(preview_x, preview_y + row_index), Print(truncate_line(line, preview_width as usize)))?;
        }
    }

    queue!(stderr, MoveTo(0, height.saturating_sub(1)), Clear(ClearType::CurrentLine))?;
    queue!(
        stderr,
        MoveTo(0, height.saturating_sub(1)),
        Print(truncate_line(
            &format!("{} matches", matched_rows.len()),
            width as usize,
        ))
    )?;

    stderr.flush()?;
    Ok(())
}

fn truncate_line(value: &str, max_width: usize) -> String {
    value.chars().take(max_width).collect()
}

#[cfg(unix)]
fn redirect_stdin_to_tty() -> io::Result<()> {
    use std::os::unix::io::AsRawFd;

    let tty = std::fs::File::open("/dev/tty")?;
    let tty_fd = tty.as_raw_fd();

    unsafe {
        if libc::dup2(tty_fd, 0) == -1 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}
