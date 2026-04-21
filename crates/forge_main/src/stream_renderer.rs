use std::io;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use colored::Colorize;
use forge_domain::ConsoleWriter;
use forge_markdown_stream::StreamdownRenderer;
use forge_spinner::SpinnerManager;

/// Shared spinner wrapper that encapsulates locking for thread-safe spinner
/// operations.
///
/// Provides the same API as `SpinnerManager` but handles mutex locking
/// internally, releasing the lock immediately after each operation completes.
pub struct SharedSpinner<P: ConsoleWriter>(Arc<Mutex<SpinnerManager<P>>>);

impl<P: ConsoleWriter> Clone for SharedSpinner<P> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<P: ConsoleWriter> SharedSpinner<P> {
    /// Creates a new shared spinner from a SpinnerManager.
    pub fn new(spinner: SpinnerManager<P>) -> Self {
        Self(Arc::new(Mutex::new(spinner)))
    }

    /// Start the spinner with a message.
    pub fn start(&self, message: Option<&str>) -> Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start(message)
    }

    /// Stop the active spinner if any.
    pub fn stop(&self, message: Option<String>) -> Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop(message)
    }

    /// Resets the stopwatch to zero.
    pub fn reset(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).reset()
    }

    /// Writes a line to stdout, suspending the spinner if active.
    pub fn write_ln(&self, message: impl ToString) -> Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_ln(message)
    }

    /// Writes a line to stderr, suspending the spinner if active.
    pub fn ewrite_ln(&self, message: impl ToString) -> Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .ewrite_ln(message)
    }
}

/// Content styling for output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Style {
    #[default]
    Normal,
    Dimmed,
}

impl Style {
    /// Applies styling to content string.
    fn apply(self, content: String) -> String {
        match self {
            Self::Normal => content,
            Self::Dimmed => content.dimmed().to_string(),
        }
    }
}

fn term_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80)
}

/// Streaming markdown writer with automatic spinner management.
///
/// Coordinates between markdown rendering and spinner visibility:
/// - Stops spinner when content is being written
/// - Restarts spinner when idle
pub struct StreamingWriter<P: ConsoleWriter> {
    active: Option<ActiveRenderer<P>>,
    spinner: SharedSpinner<P>,
    printer: Arc<P>,
}

impl<P: ConsoleWriter + 'static> StreamingWriter<P> {
    /// Creates a new stream writer with the given shared spinner and output
    /// printer.
    pub fn new(spinner: SharedSpinner<P>, printer: Arc<P>) -> Self {
        Self { active: None, spinner, printer }
    }

    /// Writes markdown content with normal styling.
    pub fn write(&mut self, text: &str) -> Result<()> {
        self.write_styled(text, Style::Normal)
    }

    /// Writes markdown content with dimmed styling (for reasoning blocks).
    pub fn write_dimmed(&mut self, text: &str) -> Result<()> {
        self.write_styled(text, Style::Dimmed)
    }

    /// Finishes any active renderer.
    pub fn finish(&mut self) -> Result<()> {
        if let Some(active) = self.active.take() {
            active.finish()?;
        }
        Ok(())
    }

    fn write_styled(&mut self, text: &str, style: Style) -> Result<()> {
        self.ensure_renderer(style)?;
        if let Some(ref mut active) = self.active {
            active.push(text)?;
        }
        Ok(())
    }

    fn ensure_renderer(&mut self, new_style: Style) -> Result<()> {
        let needs_switch = self.active.as_ref().is_some_and(|a| a.style != new_style);

        if needs_switch && let Some(old) = self.active.take() {
            old.finish()?;
        }

        if self.active.is_none() {
            let writer = StreamDirectWriter {
                spinner: self.spinner.clone(),
                printer: self.printer.clone(),
                style: new_style,
            };
            let renderer = StreamdownRenderer::new(writer, term_width());
            self.active = Some(ActiveRenderer { renderer, style: new_style });
        }
        Ok(())
    }
}

/// Active renderer with its style.
struct ActiveRenderer<P: ConsoleWriter> {
    renderer: StreamdownRenderer<StreamDirectWriter<P>>,
    style: Style,
}

impl<P: ConsoleWriter> ActiveRenderer<P> {
    pub fn push(&mut self, text: &str) -> Result<()> {
        self.renderer.push(text)?;
        Ok(())
    }

    pub fn finish(self) -> Result<()> {
        self.renderer.finish()?;
        Ok(())
    }
}

/// Writer for streamdown that outputs to printer and manages spinner.
struct StreamDirectWriter<P: ConsoleWriter> {
    spinner: SharedSpinner<P>,
    printer: Arc<P>,
    style: Style,
}

impl<P: ConsoleWriter> StreamDirectWriter<P> {
    fn pause_spinner(&self) {
        let _ = self.spinner.stop(None);
    }

    fn resume_spinner(&self) {
        let _ = self.spinner.start(None);
    }
}

impl<P: ConsoleWriter> Drop for StreamDirectWriter<P> {
    fn drop(&mut self) {
        let _ = self.printer.flush();
        let _ = self.printer.flush_err();
    }
}

impl<P: ConsoleWriter> io::Write for StreamDirectWriter<P> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pause_spinner();

        let content = match std::str::from_utf8(buf) {
            Ok(s) => s.to_string(),
            Err(_) => String::from_utf8_lossy(buf).into_owned(),
        };
        let styled = self.style.apply(content);
        self.printer.write(styled.as_bytes())?;
        self.printer.flush()?;

        // Track if we ended on a newline - only safe to show spinner at line start
        if buf.last() == Some(&b'\n') {
            self.resume_spinner();
        }

        // Return `buf.len()`, not `styled.as_bytes().len()`. The `io::Write` contract
        // requires returning how many bytes were consumed from the input buffer, not
        // how many bytes were written to the output. Styling adds ANSI escape codes
        // which makes the output larger than the input.
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.printer.flush()
    }
}
