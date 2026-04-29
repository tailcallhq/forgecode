//! Forge Markdown Stream - Streaming markdown renderer for terminal output.
//!
//! This crate provides a streaming markdown renderer optimized for LLM output.
//! It renders markdown with syntax highlighting, styled headings, tables,
//! lists, and more.
//!
//! # Example
//!
//! ```no_run
//! use forge_markdown_stream::StreamdownRenderer;
//! use std::io;
//!
//! fn main() -> io::Result<()> {
//!     let mut renderer = StreamdownRenderer::new(io::stdout(), 80);
//!
//!     // Push tokens as they arrive from LLM
//!     renderer.push("Hello ")?;
//!     renderer.push("**world**!\n")?;
//!
//!     // Finish rendering
//!     let _ = renderer.finish()?;
//!     Ok(())
//! }
//! ```

mod code;
mod heading;
mod inline;
mod list;
mod renderer;
mod repair;
mod style;
mod table;
mod theme;
mod utils;

use std::io::{self, Write};

pub use renderer::Renderer;
pub use repair::repair_line;
pub use streamdown_parser::Parser;
pub use theme::{Style, Theme};

/// Streaming markdown renderer for terminal output.
///
/// Buffers incoming tokens and renders complete lines with syntax highlighting,
/// styled headings, tables, lists, and more.
///
/// The renderer is generic over the writer type `W`, which must implement
/// `Write`.
pub struct StreamdownRenderer<W: Write> {
    parser: Parser,
    renderer: Renderer<W>,
    line_buffer: String,
}

impl<W: Write> StreamdownRenderer<W> {
    /// Create a new renderer with the given writer and terminal width.
    pub fn new(writer: W, width: usize) -> Self {
        Self {
            parser: Parser::new(),
            renderer: Renderer::new(writer, width),
            line_buffer: String::new(),
        }
    }

    /// Create a new renderer with a custom theme.
    pub fn with_theme(writer: W, width: usize, theme: Theme) -> Self {
        Self {
            parser: Parser::new(),
            renderer: Renderer::with_theme(writer, width, theme),
            line_buffer: String::new(),
        }
    }

    /// Push a token to the renderer.
    ///
    /// Tokens are buffered until a complete line is received, then rendered.
    pub fn push(&mut self, token: &str) -> io::Result<()> {
        self.line_buffer.push_str(token);

        while let Some(pos) = self.line_buffer.find('\n') {
            let line = self.line_buffer.get(..pos).unwrap_or("").to_string();

            for repaired in repair_line(&line, self.parser.state()) {
                for event in self.parser.parse_line(&repaired) {
                    self.renderer.render_event(&event)?;
                }
            }

            self.line_buffer = self.line_buffer.get(pos + 1..).unwrap_or("").to_string();
        }
        Ok(())
    }

    /// Finish rendering, flushing any remaining buffered content.
    /// Returns the underlying writer.
    pub fn finish(mut self) -> io::Result<()> {
        if !self.line_buffer.is_empty() {
            for repaired in repair_line(&self.line_buffer, self.parser.state()) {
                for event in self.parser.parse_line(&repaired) {
                    self.renderer.render_event(&event)?;
                }
            }
        }
        for event in self.parser.finalize() {
            self.renderer.render_event(&event)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::StreamdownRenderer;

    fn fixture_rendered_output(markdown: &str, width: usize) -> String {
        let mut output = Vec::new();
        let mut fixture = StreamdownRenderer::new(&mut output, width);
        fixture.push(markdown).unwrap();
        fixture.finish().unwrap();

        let actual = strip_ansi_escapes::strip(output);
        String::from_utf8(actual)
            .unwrap()
            .trim_matches('\n')
            .to_string()
    }

    fn fixture_rendered_output_from_chunks(chunks: &[&str], width: usize) -> String {
        let mut output = Vec::new();
        let mut fixture = StreamdownRenderer::new(&mut output, width);
        for chunk in chunks {
            fixture.push(chunk).unwrap();
        }
        fixture.finish().unwrap();

        let actual = strip_ansi_escapes::strip(output);
        String::from_utf8(actual)
            .unwrap()
            .trim_matches('\n')
            .to_string()
    }

    #[test]
    fn test_streaming_renderer_preserves_korean_spacing_in_structured_markdown() {
        let fixture = concat!(
            "## 구현 요약\n",
            "- 각 서비스에서 metadata key를 개별 수정하지 않고, object storage 공통 레이어에서 일괄 정규화하도록 반영했습니다.\n",
            "## 검토 사항\n",
            "- 본 수정은 업로드 시 metadata header 이름 문제를 해결합니다.\n",
            "- 추가적인 권한 정책, bucket policy, reverse proxy 제한이 있으면 별도 오류가 발생할 수 있습니다.\n",
        );
        let actual = fixture_rendered_output(fixture, 200);
        let expected = concat!(
            "## 구현 요약\n",
            "• 각 서비스에서 metadata key를 개별 수정하지 않고, object storage 공통 레이어에서 일괄 정규화하도록 반영했습니다.\n",
            "\n",
            "## 검토 사항\n",
            "• 본 수정은 업로드 시 metadata header 이름 문제를 해결합니다.\n",
            "• 추가적인 권한 정책, bucket policy, reverse proxy 제한이 있으면 별도 오류가 발생할 수 있습니다.",
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_streaming_renderer_preserves_korean_spacing_when_structured_tail_arrives_in_chunks() {
        let fixture = [
            "## 검토 결과\n",
            "- 본 사례는 스트리밍 마크다운 렌더링의 공백 재조합 문제와 관련이 있습니다.\n",
            "- 핵심 구현은 공백 보존 래퍼에 위치합니다.\n",
            "- 회귀 테스트는 스트리밍 렌더러 검증 항목에 추가되어 있습니다.\n\n",
            "후속 작업은 다음과 같습니다.\n",
            "1. 변경 사항을 검토 가능한 형식으로 정리합니다.\n",
            "2. 실제 대화 출력과 유사한 통합 테스트 범위를 ",
            "확장합니다.",
        ];
        let actual = fixture_rendered_output_from_chunks(&fixture, 200);
        let expected = concat!(
            "## 검토 결과\n",
            "• 본 사례는 스트리밍 마크다운 렌더링의 공백 재조합 문제와 관련이 있습니다.\n",
            "• 핵심 구현은 공백 보존 래퍼에 위치합니다.\n",
            "• 회귀 테스트는 스트리밍 렌더러 검증 항목에 추가되어 있습니다.\n",
            "\n",
            "후속 작업은 다음과 같습니다.\n",
            "1. 변경 사항을 검토 가능한 형식으로 정리합니다.\n",
            "2. 실제 대화 출력과 유사한 통합 테스트 범위를 확장합니다.",
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_streaming_renderer_wraps_blockquotes_with_prefix_width_and_long_tokens() {
        let fixture = "> supercalifragilistic\n> 한글 공백\n";
        let actual = fixture_rendered_output(fixture, 10);
        let expected = concat!(
            "│ supercal\n",
            "│ ifragili\n",
            "│ stic\n",
            "│ 한글\n",
            "│ 공백"
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_streaming_renderer_wraps_blockquote_links_without_losing_separator() {
        let fixture = "> [링크](https://example.com/very/long/path) 설명\n";
        let actual = fixture_rendered_output(fixture, 20);
        let expected = concat!(
            "│ 링크\n",
            "│ (https://example.c\n",
            "│ om/very/long/path)\n",
            "│ 설명"
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_streaming_renderer_wraps_nested_blockquotes_with_correct_prefix_width() {
        let fixture = ">> supercalifragilistic\n";
        let actual = fixture_rendered_output(fixture, 12);
        let expected = concat!("│ │ supercal\n", "│ │ ifragili\n", "│ │ stic");

        assert_eq!(actual, expected);
    }
}
