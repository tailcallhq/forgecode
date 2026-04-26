use nom::Parser;
use nom::bytes::complete::tag;

use crate::{FileInfo, Image};

/// A file or directory attachment included in a chat message.
#[derive(Debug, serde::Deserialize, serde::Serialize, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// The resolved content of the attachment (image, file text, or directory
    /// listing).
    pub content: AttachmentContent,
    /// The original path or URL string used to reference this attachment.
    pub path: String,
}

/// The resolved content of an attachment, discriminated by the type of resource
/// it represents.
#[derive(Debug, serde::Deserialize, serde::Serialize, Clone, PartialEq, Eq)]
pub enum AttachmentContent {
    /// A binary image file encoded for inline display.
    Image(Image),
    /// A text file, optionally restricted to a line range.
    FileContent {
        /// Line-numbered display text shown to the model. May represent only a
        /// slice of the full file when a range was requested.
        content: String,
        /// Metadata about the file read: line positions and full-file content
        /// hash for external-change detection.
        info: FileInfo,
    },
    /// A directory listing showing the immediate children of a directory.
    DirectoryListing {
        /// Entries contained in the directory.
        entries: Vec<DirectoryEntry>,
    },
}

/// A single entry within a directory listing attachment.
#[derive(Debug, serde::Deserialize, serde::Serialize, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    /// Path of the entry relative to the listed directory.
    pub path: String,
    /// Whether this entry is itself a directory.
    pub is_dir: bool,
}

impl AttachmentContent {
    pub fn as_image(&self) -> Option<&Image> {
        match self {
            AttachmentContent::Image(image) => Some(image),
            _ => None,
        }
    }

    pub fn contains(&self, text: &str) -> bool {
        match self {
            AttachmentContent::Image(_) => false,
            AttachmentContent::FileContent { content, .. } => content.contains(text),
            AttachmentContent::DirectoryListing { .. } => false,
        }
    }

    pub fn file_content(&self) -> Option<&str> {
        match self {
            AttachmentContent::FileContent { content, .. } => Some(content),
            _ => None,
        }
    }

    pub fn range_info(&self) -> Option<(u64, u64, u64)> {
        match self {
            AttachmentContent::FileContent { info, .. } => {
                Some((info.start_line, info.end_line, info.total_lines))
            }
            _ => None,
        }
    }
}

impl Attachment {
    /// Parses a string and extracts all file paths in the format
    /// @[path/to/file]. File paths can contain spaces and are considered to
    /// extend until the closing bracket. If the closing bracket is missing,
    /// consider everything until the end of the string as the path.
    pub fn parse_all<T: ToString>(text: T) -> Vec<FileTag> {
        let input = text.to_string();
        let mut remaining = input.as_str();
        let mut tags = Vec::new();

        while !remaining.is_empty() {
            // Find the next "@[" pattern
            if let Some(start_pos) = remaining.find("@[") {
                // Move to the position where "@[" starts
                remaining = remaining.get(start_pos..).unwrap_or("");
                match FileTag::parse(remaining) {
                    Ok((next_remaining, file_tag)) => {
                        tags.push(file_tag);
                        remaining = next_remaining;
                    }
                    Err(_e) => {
                        // Skip the "@[" since we couldn't parse it
                        remaining = remaining.get(2..).unwrap_or("");
                    }
                }
            } else {
                // No more "@[" patterns found
                break;
            }
        }

        let mut seen = std::collections::HashSet::new();
        tags.retain(|tag| seen.insert((tag.path.clone(), tag.loc.clone(), tag.symbol.clone())));

        tags
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    pub start: Option<u64>,
    pub end: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileTag {
    pub path: String,
    pub loc: Option<Location>,
    pub symbol: Option<String>,
}

impl FileTag {
    pub fn parse(input: &str) -> nom::IResult<&str, FileTag> {
        use nom::bytes::complete::take_while1;
        use nom::character::complete::{char, digit1};
        use nom::combinator::{map_res, opt};
        use nom::sequence::{delimited, preceded};

        let parse_u64 = || map_res(digit1, str::parse::<u64>);
        let parse_symbol = preceded(char('#'), take_while1(|c: char| c != ']'));

        let parse_location_full = (
            preceded(char(':'), parse_u64()),
            preceded(char(':'), parse_u64()),
        );
        let parse_location_start_only = preceded(char(':'), parse_u64());

        let parse_location = nom::branch::alt((
            nom::combinator::map(parse_location_full, |(start, end)| (Some(start), Some(end))),
            nom::combinator::map(parse_location_start_only, |start| (Some(start), None)),
        ));

        let parse_path = nom::branch::alt((
            // Try Windows drive path first (letter:path)
            nom::combinator::recognize((
                nom::character::complete::satisfy(|c| c.is_ascii_alphabetic()),
                nom::character::complete::char(':'),
                take_while1(|c: char| c != ':' && c != '#' && c != ']'),
            )),
            // Fall back to regular path parsing
            take_while1(|c: char| c != ':' && c != '#' && c != ']'),
        ));
        let mut parser = delimited(
            tag("@["),
            (parse_path, opt(parse_location), opt(parse_symbol)),
            char(']'),
        );

        let (remaining, (path, location, symbol)) = parser.parse(input)?;
        let loc = location.map(|(start, end)| Location { start, end });
        Ok((
            remaining,
            FileTag {
                path: path.to_string(),
                loc,
                symbol: symbol.map(|s| s.to_string()),
            },
        ))
    }
}

impl AsRef<std::path::Path> for FileTag {
    fn as_ref(&self) -> &std::path::Path {
        std::path::Path::new(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_attachment_parse_all_empty() {
        let text = String::from("No attachments here");
        let attachments = Attachment::parse_all(text);
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_attachment_parse_all_simple() {
        let text = String::from("Check this file @[/path/to/file.txt]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let path_found = paths.first().unwrap();
        assert_eq!(path_found.path, "/path/to/file.txt");
    }

    #[test]
    fn test_attachment_parse_all_with_spaces() {
        let text = String::from("Check this file @[/path/with spaces/file.txt]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let path_found = paths.first().unwrap();
        assert_eq!(path_found.path, "/path/with spaces/file.txt");
    }

    #[test]
    fn test_attachment_parse_all_multiple() {
        let text = String::from(
            "Check @[/file1.txt] and also @[/path/with spaces/file2.txt] and @[/file3.txt]",
        );
        let paths = Attachment::parse_all(text);
        let paths = paths
            .iter()
            .map(|tag| tag.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), 3);

        assert!(paths.contains(&"/file1.txt"));
        assert!(paths.contains(&"/path/with spaces/file2.txt"));
        assert!(paths.contains(&"/file3.txt"));
    }

    #[test]
    fn test_attachment_parse_all_at_end() {
        let text = String::from("Check this file @[");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 0);
    }

    #[test]
    fn test_attachment_parse_all_unclosed_bracket() {
        let text = String::from("Check this file @[/path/with spaces/unclosed");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 0);
    }

    #[test]
    fn test_attachment_parse_all_with_multibyte_chars() {
        let text = String::from(
            "Check this file @[🚀/path/with spaces/file.txt🔥] and also @[🌟simple_path]",
        );
        let paths = Attachment::parse_all(text);
        let paths = paths
            .iter()
            .map(|tag| tag.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), 2);

        assert!(paths.contains(&"🚀/path/with spaces/file.txt🔥"));
        assert!(paths.contains(&"🌟simple_path"));
    }

    #[test]
    fn test_attachment_parse_with_location() {
        let text = String::from("Check line @[/path/to/file.txt:10:20]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/path/to/file.txt".to_string(),
            loc: Some(Location { start: Some(10), end: Some(20) }),
            symbol: None,
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_with_symbol() {
        let text = String::from("Check function @[/path/to/file.rs#my_function]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/path/to/file.rs".to_string(),
            loc: None,
            symbol: Some("my_function".to_string()),
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_with_location_and_symbol() {
        let text = String::from("Check @[/src/main.rs:5:15#main_function]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/src/main.rs".to_string(),
            loc: Some(Location { start: Some(5), end: Some(15) }),
            symbol: Some("main_function".to_string()),
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_multiple_with_mixed_features() {
        let text = String::from(
            "Check @[/file1.txt] and @[/file2.rs:10:20] and @[/file3.py#function] and @[/file4.js:1:5#init]",
        );
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 4);

        let expected = vec![
            FileTag { path: "/file1.txt".to_string(), loc: None, symbol: None },
            FileTag {
                path: "/file2.rs".to_string(),
                loc: Some(Location { start: Some(10), end: Some(20) }),
                symbol: None,
            },
            FileTag {
                path: "/file3.py".to_string(),
                loc: None,
                symbol: Some("function".to_string()),
            },
            FileTag {
                path: "/file4.js".to_string(),
                loc: Some(Location { start: Some(1), end: Some(5) }),
                symbol: Some("init".to_string()),
            },
        ];

        for expected_tag in expected {
            assert!(paths.contains(&expected_tag));
        }
    }

    #[test]
    fn test_attachment_parse_symbol_with_special_chars() {
        let text = String::from("Check @[/file.rs#function_with_underscore_123]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/file.rs".to_string(),
            loc: None,
            symbol: Some("function_with_underscore_123".to_string()),
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_location_edge_cases() {
        let text = String::from("Check @[/file.txt:0:999999]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/file.txt".to_string(),
            loc: Some(Location { start: Some(0), end: Some(999999) }),
            symbol: None,
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_location_with_start() {
        let text = String::from("Check @[/file.txt:12#main()]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/file.txt".to_string(),
            loc: Some(Location { start: Some(12), end: None }),
            symbol: Some("main()".to_string()),
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_location_duplicate_entries() {
        let text = String::from("Check @[/file.txt:12#main()] and @[/file.txt:12#main()]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/file.txt".to_string(),
            loc: Some(Location { start: Some(12), end: None }),
            symbol: Some("main()".to_string()),
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_windows_drive_path() {
        let text = String::from("Check @[C:\\Users\\test\\file.txt:10:20]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "C:\\Users\\test\\file.txt".to_string(),
            loc: Some(Location { start: Some(10), end: Some(20) }),
            symbol: None,
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_windows_drive_simple() {
        let text = String::from("Check @[D:\\file.txt]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag { path: "D:\\file.txt".to_string(), loc: None, symbol: None };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_windows_drive_with_symbol() {
        let text = String::from("Check @[E:\\src\\main.rs#function_name]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "E:\\src\\main.rs".to_string(),
            loc: None,
            symbol: Some("function_name".to_string()),
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_windows_drive_with_line_start_only() {
        let text = String::from("Check @[F:\\project\\lib.rs:42]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "F:\\project\\lib.rs".to_string(),
            loc: Some(Location { start: Some(42), end: None }),
            symbol: None,
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_windows_drive_with_line_range_and_symbol() {
        let text = String::from("Check @[G:\\code\\test.rs:5:15#test_function]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "G:\\code\\test.rs".to_string(),
            loc: Some(Location { start: Some(5), end: Some(15) }),
            symbol: Some("test_function".to_string()),
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_linux_path_with_line_numbers() {
        let text = String::from("Check @[/home/user/project/file.rs:25:30]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/home/user/project/file.rs".to_string(),
            loc: Some(Location { start: Some(25), end: Some(30) }),
            symbol: None,
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_linux_path_with_line_start_only() {
        let text = String::from("Check @[/var/log/app.log:100]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/var/log/app.log".to_string(),
            loc: Some(Location { start: Some(100), end: None }),
            symbol: None,
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_unix_path_simple() {
        let text = String::from("Check @[/usr/local/bin/app]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/usr/local/bin/app".to_string(),
            loc: None,
            symbol: None,
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_unix_path_with_symbol() {
        let text = String::from("Check @[/opt/project/src/main.c#main]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/opt/project/src/main.c".to_string(),
            loc: None,
            symbol: Some("main".to_string()),
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_unix_path_with_line_and_symbol() {
        let text = String::from("Check @[/tmp/script.sh:10#setup_function]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 1);

        let expected = FileTag {
            path: "/tmp/script.sh".to_string(),
            loc: Some(Location { start: Some(10), end: None }),
            symbol: Some("setup_function".to_string()),
        };
        let actual = paths.first().unwrap();
        assert_eq!(actual, &expected);
    }

    #[test]
    fn test_attachment_parse_mixed_unix_and_windows() {
        let text = String::from("Check @[/unix/path.txt] and @[C:\\windows\\path.txt]");
        let paths = Attachment::parse_all(text);
        assert_eq!(paths.len(), 2);

        let expected_unix = FileTag { path: "/unix/path.txt".to_string(), loc: None, symbol: None };
        let expected_windows = FileTag {
            path: "C:\\windows\\path.txt".to_string(),
            loc: None,
            symbol: None,
        };

        assert!(paths.contains(&expected_unix));
        assert!(paths.contains(&expected_windows));
    }
}
