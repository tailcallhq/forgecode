use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::Result;
use colored::Colorize;
use forge_domain::ConsoleWriter;
use rand::RngExt;

mod progress_bar;

pub use progress_bar::*;

const TICK_DURATION_MS: u64 = 60;
const TICKS: &[&str; 10] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MIN_TERMINAL_WIDTH: usize = 12;
const WRAP_GUARD_COLUMNS: usize = 8;

fn terminal_width() -> usize {
    terminal_size::terminal_size()
        .map(|(width, _)| width.0 as usize)
        .unwrap_or(80)
}

fn visible_width(value: &str) -> usize {
    console::measure_text_width(value)
}

fn truncate_to_visible_width(value: &str, max_width: usize) -> String {
    let mut output = value.to_string();

    while visible_width(&output) > max_width {
        if output.pop().is_none() {
            break;
        }
    }

    output
}

fn styled_loader_line(
    tick: &str,
    message: &str,
    elapsed: Duration,
    terminal_width: usize,
) -> String {
    let elapsed = format_elapsed_time(elapsed);
    let suffix = "· Ctrl+C to interrupt";
    let max_width = terminal_width
        .saturating_sub(WRAP_GUARD_COLUMNS)
        .max(MIN_TERMINAL_WIDTH);

    let tick = tick.green().to_string();
    let elapsed = elapsed.white().to_string();
    let suffix = suffix.white().dimmed().to_string();
    let fixed = format!("{tick}  {elapsed} {suffix}");
    let message_width = max_width.saturating_sub(visible_width(&fixed)).max(1);
    let message = truncate_to_visible_width(message, message_width)
        .green()
        .bold()
        .to_string();
    let styled = format!("{tick} {message} {elapsed} {suffix}");

    truncate_to_visible_width(&styled, max_width)
}

struct ActiveSpinner<P: ConsoleWriter> {
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    started_at: Instant,
    accumulated_elapsed: Duration,
    printer: Arc<P>,
}

impl<P: ConsoleWriter + 'static> ActiveSpinner<P> {
    fn start(printer: Arc<P>, accumulated_elapsed: Duration, message: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let stop_signal = Arc::clone(&stop);
        let paused_signal = Arc::clone(&paused);
        let thread_printer = Arc::clone(&printer);
        let started_at = Instant::now();

        let handle = thread::spawn(move || {
            loop {
                if stop_signal.load(Ordering::Acquire) {
                    break;
                }

                if paused_signal.load(Ordering::Acquire) {
                    thread::park_timeout(Duration::from_millis(TICK_DURATION_MS));
                    continue;
                }

                let elapsed = accumulated_elapsed + started_at.elapsed();
                let tick_index = ((elapsed.as_millis() / TICK_DURATION_MS as u128)
                    % TICKS.len() as u128) as usize;
                let tick = TICKS.get(tick_index).unwrap_or(&"⠋");
                let line = styled_loader_line(tick, &message, elapsed, terminal_width());

                if !stop_signal.load(Ordering::Acquire) && !paused_signal.load(Ordering::Acquire) {
                    let _ = thread_printer.write_err(format!("\r\x1b[2K{line}").as_bytes());
                    let _ = thread_printer.flush_err();
                }

                thread::park_timeout(Duration::from_millis(TICK_DURATION_MS));
            }
        });

        Self {
            stop,
            paused,
            handle: Some(handle),
            started_at,
            accumulated_elapsed,
            printer,
        }
    }
}

impl<P: ConsoleWriter> ActiveSpinner<P> {
    fn elapsed(&self) -> Duration {
        self.accumulated_elapsed + self.started_at.elapsed()
    }

    fn pause(&self) {
        let was_paused = self.paused.swap(true, Ordering::AcqRel);
        if !was_paused {
            self.clear_line();
        }
        if let Some(handle) = &self.handle {
            handle.thread().unpark();
        }
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        if let Some(handle) = &self.handle {
            handle.thread().unpark();
        }
    }

    fn clear_line(&self) {
        let _ = self.printer.write_err(b"\r\x1b[2K");
        let _ = self.printer.flush_err();
    }

    fn finish(&mut self) -> Duration {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = &self.handle {
            handle.thread().unpark();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let elapsed = self.elapsed();
        self.clear_line();
        elapsed
    }
}

impl<P: ConsoleWriter> Drop for ActiveSpinner<P> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = &self.handle {
            handle.thread().unpark();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.clear_line();
    }
}

/// Formats elapsed time into a compact string representation.
///
/// # Arguments
///
/// * `duration` - The elapsed time duration
///
/// # Returns
///
/// A formatted string:
/// - Less than 1 minute: "01s", "02s", etc.
/// - Less than 1 hour: "1:01m", "1:59m", etc.
/// - 1 hour or more: "1:01h", "2:30h", etc.
fn format_elapsed_time(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    if total_seconds < 60 {
        format!("{:02}s", total_seconds)
    } else if total_seconds < 3600 {
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        format!("{}:{:02}m", minutes, seconds)
    } else {
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        format!("{}:{:02}h", hours, minutes)
    }
}

/// Manages spinner functionality for the UI.
///
/// Renders the loader through a resize-safe manual terminal writer that clears
/// and redraws a single truncated line on each tick. Accumulated time is
/// preserved across start/stop cycles so paused output can resume without
/// resetting the elapsed timer.
pub struct SpinnerManager<P: ConsoleWriter> {
    spinner: Option<ActiveSpinner<P>>,
    accumulated_elapsed: Duration,
    word_index: Option<usize>,
    message: Option<String>,
    printer: Arc<P>,
}

impl<P: ConsoleWriter + 'static> SpinnerManager<P> {
    /// Creates a new SpinnerManager with the given output printer.
    pub fn new(printer: Arc<P>) -> Self {
        Self {
            spinner: None,
            accumulated_elapsed: Duration::ZERO,
            word_index: None,
            message: None,
            printer,
        }
    }

    /// Start the spinner with a message
    pub fn start(&mut self, message: Option<&str>) -> Result<()> {
        self.stop(None)?;

        let words = [
            "Thinking",
            "Processing",
            "Analyzing",
            "Forging",
            "Researching",
            "Synthesizing",
            "Reasoning",
            "Contemplating",
        ];

        // Use a random word from the list, caching the index for consistency
        let word = match message {
            Some(msg) => msg.to_string(),
            None => {
                let idx = *self
                    .word_index
                    .get_or_insert_with(|| rand::rng().random_range(0..words.len()));
                words.get(idx).unwrap_or(&"Loading").to_string()
            }
        };

        self.message = Some(word.clone());

        let spinner = ActiveSpinner::start(self.printer.clone(), self.accumulated_elapsed, word);
        self.spinner = Some(spinner);

        Ok(())
    }

    /// Pauses the active spinner without stopping its render thread.
    pub fn pause(&mut self) {
        if let Some(spinner) = &self.spinner {
            spinner.pause();
        }
    }

    /// Resumes a previously paused spinner.
    pub fn resume(&mut self) {
        if let Some(spinner) = &self.spinner {
            spinner.resume();
        }
    }

    /// Stop the active spinner if any
    pub fn stop(&mut self, message: Option<String>) -> Result<()> {
        if let Some(mut spinner) = self.spinner.take() {
            // Capture elapsed time before finishing
            self.accumulated_elapsed = spinner.finish();
            if let Some(msg) = message {
                self.println(&msg);
            }
        } else if let Some(message) = message {
            self.println(&message);
        }

        self.message = None;

        Ok(())
    }

    /// Updates the spinner's displayed message.
    pub fn set_message(&mut self, message: &str) -> Result<()> {
        self.message = Some(message.to_owned());
        if self.spinner.is_some() {
            self.stop(None)?;
            self.start(Some(message))?;
        }
        Ok(())
    }

    /// Resets the elapsed time to zero.
    /// Call this when starting a completely new task/conversation.
    pub fn reset(&mut self) {
        self.accumulated_elapsed = Duration::ZERO;
        self.word_index = None;
        self.message = None;
    }

    /// Writes a line to stdout, suspending the spinner if active.
    pub fn write_ln(&mut self, message: impl ToString) -> Result<()> {
        let msg = message.to_string();
        let was_active = self.spinner.is_some();
        if was_active {
            self.pause();
        }
        self.println(&msg);
        if was_active {
            self.resume();
        }
        Ok(())
    }

    /// Writes a line to stderr, suspending the spinner if active.
    pub fn ewrite_ln(&mut self, message: impl ToString) -> Result<()> {
        let msg = message.to_string();
        let was_active = self.spinner.is_some();
        if was_active {
            self.pause();
        }
        self.eprintln(&msg);
        if was_active {
            self.resume();
        }
        Ok(())
    }

    /// Prints a line to stdout through the printer.
    fn println(&self, msg: &str) {
        let line = format!("{msg}\n");
        let _ = self.printer.write(line.as_bytes());
        let _ = self.printer.flush();
    }

    /// Prints a line to stderr through the printer.
    fn eprintln(&self, msg: &str) {
        let line = format!("{msg}\n");
        let _ = self.printer.write_err(line.as_bytes());
        let _ = self.printer.flush_err();
    }
}

impl<P: ConsoleWriter> Drop for SpinnerManager<P> {
    fn drop(&mut self) {
        // Stop spinner before flushing to ensure finish_and_clear() is called.
        if let Some(mut spinner) = self.spinner.take() {
            self.accumulated_elapsed = spinner.finish();
        }
        // Flush both stdout and stderr to ensure all output is visible
        // This prevents race conditions with shell prompt resets
        let _ = self.printer.flush();
        let _ = self.printer.flush_err();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use forge_domain::ConsoleWriter;
    use pretty_assertions::assert_eq;

    use super::{SpinnerManager, format_elapsed_time};

    /// A simple printer that writes directly to stdout/stderr.
    /// Used for testing when synchronized output is not needed.
    #[derive(Clone, Copy)]
    struct DirectPrinter;

    impl ConsoleWriter for DirectPrinter {
        fn write(&self, buf: &[u8]) -> std::io::Result<usize> {
            std::io::stdout().write(buf)
        }

        fn write_err(&self, buf: &[u8]) -> std::io::Result<usize> {
            std::io::stderr().write(buf)
        }

        fn flush(&self) -> std::io::Result<()> {
            std::io::stdout().flush()
        }

        fn flush_err(&self) -> std::io::Result<()> {
            std::io::stderr().flush()
        }
    }

    fn fixture_spinner() -> SpinnerManager<DirectPrinter> {
        SpinnerManager::new(Arc::new(DirectPrinter))
    }

    #[test]
    fn test_spinner_reset_clears_accumulated_time() {
        let mut fixture_spinner = fixture_spinner();

        // Simulate some accumulated time
        fixture_spinner.accumulated_elapsed = std::time::Duration::from_secs(100);

        // Reset should clear accumulated time
        fixture_spinner.reset();

        let actual = fixture_spinner.accumulated_elapsed;
        let expected = std::time::Duration::ZERO;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_spinner_reset_clears_word_index() {
        let mut fixture_spinner = fixture_spinner();

        // Set a word index
        fixture_spinner.word_index = Some(3);

        // Reset should clear it
        fixture_spinner.reset();

        let actual = fixture_spinner.word_index;
        let expected = None;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_spinner_reset_clears_message() {
        let mut fixture_spinner = fixture_spinner();

        // Set a message
        fixture_spinner.message = Some("Test".to_string());

        // Reset should clear it
        fixture_spinner.reset();

        let actual = fixture_spinner.message.clone();
        let expected = None;
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_word_index_caching_behavior() {
        let mut fixture_spinner = fixture_spinner();

        // Start spinner without message multiple times
        fixture_spinner.start(None).unwrap();
        let first_index = fixture_spinner.word_index;
        fixture_spinner.stop(None).unwrap();

        fixture_spinner.start(None).unwrap();
        let second_index = fixture_spinner.word_index;
        fixture_spinner.stop(None).unwrap();

        // Word index should be identical because it's cached
        assert_eq!(first_index, second_index);
    }

    #[test]
    fn test_spinner_pause_resume_keeps_same_active_spinner() {
        let mut fixture_spinner = fixture_spinner();
        fixture_spinner.start(Some("Thinking")).unwrap();

        fixture_spinner.pause();
        let was_paused = fixture_spinner
            .spinner
            .as_ref()
            .map(|spinner| spinner.paused.load(Ordering::Acquire));
        fixture_spinner.resume();
        let was_resumed = fixture_spinner
            .spinner
            .as_ref()
            .map(|spinner| !spinner.paused.load(Ordering::Acquire));
        let actual = (was_paused, was_resumed);
        fixture_spinner.stop(None).unwrap();

        let expected = (Some(true), Some(true));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_elapsed_time_seconds_only() {
        let actual = format_elapsed_time(Duration::from_secs(5));
        let expected = "05s";
        assert_eq!(actual, expected);

        let actual = format_elapsed_time(Duration::from_secs(59));
        let expected = "59s";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_elapsed_time_minutes_and_seconds() {
        let actual = format_elapsed_time(Duration::from_secs(60));
        let expected = "1:00m";
        assert_eq!(actual, expected);

        let actual = format_elapsed_time(Duration::from_secs(125));
        let expected = "2:05m";
        assert_eq!(actual, expected);

        let actual = format_elapsed_time(Duration::from_secs(3599));
        let expected = "59:59m";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_elapsed_time_hours_and_minutes() {
        let actual = format_elapsed_time(Duration::from_secs(3600));
        let expected = "1:00h";
        assert_eq!(actual, expected);

        let actual = format_elapsed_time(Duration::from_secs(3661));
        let expected = "1:01h";
        assert_eq!(actual, expected);

        let actual = format_elapsed_time(Duration::from_secs(7200));
        let expected = "2:00h";
        assert_eq!(actual, expected);

        let actual = format_elapsed_time(Duration::from_secs(9000));
        let expected = "2:30h";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_elapsed_time_zero() {
        let actual = format_elapsed_time(Duration::ZERO);
        let expected = "00s";
        assert_eq!(actual, expected);
    }
}
