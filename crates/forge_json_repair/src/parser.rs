use serde::Deserialize;

use crate::error::{JsonRepairError, Result};

pub struct JsonRepairParser {
    chars: Vec<char>,
    i: usize,
    output: String,
}

impl JsonRepairParser {
    pub fn new(text: String) -> Self {
        Self { chars: text.chars().collect(), i: 0, output: String::new() }
    }

    pub fn parse<De: for<'de> Deserialize<'de>>(mut self) -> Result<De> {
        self.parse_markdown_code_block(&["```", "[```", "{```"]);

        let processed = self.parse_value()?;
        if !processed {
            return Err(JsonRepairError::UnexpectedEnd { position: self.chars.len() });
        }

        self.parse_markdown_code_block(&["```", "```]", "```}"]);

        let processed_comma = self.parse_character(',');
        if processed_comma {
            self.parse_whitespace_and_skip_comments(true);
        }

        if self.is_start_of_value(self.current_char()) && self.ends_with_comma_or_newline() {
            if !processed_comma {
                self.output = self.insert_before_last_whitespace(",");
            }
            self.parse_newline_delimited_json()?;
        } else if processed_comma {
            self.output = self.strip_last_occurrence(",");
        }

        // repair redundant end quotes
        while matches!(self.current_char(), Some('}' | ']')) {
            self.i += 1;
            self.parse_whitespace_and_skip_comments(true);
        }

        if self.i >= self.chars.len() {
            return Ok(serde_json::from_str::<De>(&self.output)?);
        }

        Err(JsonRepairError::UnexpectedCharacter {
            character: self.current_char().unwrap_or('\0'),
            position: self.i,
        })
    }

    fn current_char(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn parse_value(&mut self) -> Result<bool> {
        self.parse_whitespace_and_skip_comments(true);

        let processed = self.parse_object()?
            || self.parse_array()?
            || self.parse_string(false, None)?
            || self.parse_number()?
            || self.parse_keywords()
            || self.parse_unquoted_string(false)?
            || self.parse_regex()?;

        self.parse_whitespace_and_skip_comments(true);
        Ok(processed)
    }

    fn parse_whitespace_and_skip_comments(&mut self, skip_newline: bool) -> bool {
        let start = self.i;

        self.parse_whitespace(skip_newline);
        loop {
            if self.parse_comment() {
                self.parse_whitespace(skip_newline);
            } else {
                break;
            }
        }

        self.i > start
    }

    fn parse_whitespace(&mut self, skip_newline: bool) -> bool {
        let mut whitespace = String::new();

        while let Some(ch) = self.current_char() {
            if self.is_whitespace(ch, skip_newline) {
                whitespace.push(ch);
                self.i += 1;
            } else if self.is_special_whitespace(ch) {
                whitespace.push(' ');
                self.i += 1;
            } else {
                break;
            }
        }

        if !whitespace.is_empty() {
            self.output.push_str(&whitespace);
            true
        } else {
            false
        }
    }

    fn parse_comment(&mut self) -> bool {
        // block comment /* ... */
        if self.current_char() == Some('/') && self.chars.get(self.i + 1) == Some(&'*') {
            while self.i < self.chars.len() && !self.at_end_of_block_comment() {
                self.i += 1;
            }
            self.i += 2;
            return true;
        }

        // line comment // ...
        if self.current_char() == Some('/') && self.chars.get(self.i + 1) == Some(&'/') {
            while self.i < self.chars.len() && self.current_char() != Some('\n') {
                self.i += 1;
            }
            return true;
        }

        false
    }

    fn parse_markdown_code_block(&mut self, blocks: &[&str]) -> bool {
        if self.skip_markdown_code_block(blocks) {
            if self.is_function_name_char_start(self.current_char()) {
                while self.i < self.chars.len() && self.is_function_name_char(self.current_char()) {
                    self.i += 1;
                }
            }
            self.parse_whitespace_and_skip_comments(true);
            return true;
        }
        false
    }

    fn skip_markdown_code_block(&mut self, blocks: &[&str]) -> bool {
        for block in blocks {
            let block_chars: Vec<char> = block.chars().collect();
            if self.i + block_chars.len() <= self.chars.len() {
                let slice = &self.chars.get(self.i..self.i + block_chars.len());
                if slice == &Some(block_chars.as_slice()) {
                    self.i += block_chars.len();
                    return true;
                }
            }
        }
        false
    }

    fn parse_character(&mut self, expected: char) -> bool {
        if self.current_char() == Some(expected) {
            self.output.push(expected);
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn skip_character(&mut self, expected: char) -> bool {
        if self.current_char() == Some(expected) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn skip_ellipsis(&mut self) -> bool {
        self.parse_whitespace_and_skip_comments(true);

        if self.current_char() == Some('.')
            && self.chars.get(self.i + 1) == Some(&'.')
            && self.chars.get(self.i + 2) == Some(&'.')
        {
            self.i += 3;
            self.parse_whitespace_and_skip_comments(true);
            self.skip_character(',');
            return true;
        }
        false
    }

    fn parse_object(&mut self) -> Result<bool> {
        if self.current_char() != Some('{') {
            return Ok(false);
        }

        self.output.push('{');
        self.i += 1;
        self.parse_whitespace_and_skip_comments(true);

        // repair: skip leading comma like in {, message: "hi"}
        if self.skip_character(',') {
            self.parse_whitespace_and_skip_comments(true);
        }

        let mut initial = true;
        while self.i < self.chars.len() && self.current_char() != Some('}') {
            let _processed_comma = if !initial {
                let comma = self.parse_character(',');
                if !comma {
                    // repair missing comma
                    self.output = self.insert_before_last_whitespace(",");
                }
                self.parse_whitespace_and_skip_comments(true);
                comma
            } else {
                initial = false;
                true
            };

            self.skip_ellipsis();

            let processed_key =
                self.parse_string(false, None)? || self.parse_unquoted_string(true)?;
            if !processed_key {
                if matches!(self.current_char(), Some('}' | '{' | ']' | '[') | None) {
                    // repair trailing comma
                    self.output = self.strip_last_occurrence(",");
                } else {
                    return Err(JsonRepairError::ObjectKeyExpected { position: self.i });
                }
                break;
            }

            self.parse_whitespace_and_skip_comments(true);
            let processed_colon = self.parse_character(':');
            let truncated_text = self.i >= self.chars.len();

            if !processed_colon {
                if self.is_start_of_value(self.current_char()) || truncated_text {
                    // repair missing colon
                    self.output = self.insert_before_last_whitespace(":");
                } else {
                    return Err(JsonRepairError::ColonExpected { position: self.i });
                }
            }

            let processed_value = self.parse_value()?;
            if !processed_value {
                if processed_colon || truncated_text {
                    // repair missing object value
                    self.output.push_str("null");
                } else {
                    return Err(JsonRepairError::ColonExpected { position: self.i });
                }
            }
        }

        if self.current_char() == Some('}') {
            self.output.push('}');
            self.i += 1;
        } else {
            // repair missing end bracket
            self.output = self.insert_before_last_whitespace("}");
        }

        Ok(true)
    }

    fn parse_array(&mut self) -> Result<bool> {
        if self.current_char() != Some('[') {
            return Ok(false);
        }

        self.output.push('[');
        self.i += 1;
        self.parse_whitespace_and_skip_comments(true);

        // repair: skip leading comma like in [,1,2,3]
        if self.skip_character(',') {
            self.parse_whitespace_and_skip_comments(true);
        }

        let mut initial = true;
        while self.i < self.chars.len() && self.current_char() != Some(']') {
            if !initial {
                let processed_comma = self.parse_character(',');
                if !processed_comma {
                    // repair missing comma
                    self.output = self.insert_before_last_whitespace(",");
                }
            } else {
                initial = false;
            }

            self.skip_ellipsis();

            let processed_value = self.parse_value()?;
            if !processed_value {
                // repair trailing comma
                self.output = self.strip_last_occurrence(",");
                break;
            }
        }

        if self.current_char() == Some(']') {
            self.output.push(']');
            self.i += 1;
        } else {
            // repair missing closing array bracket
            self.output = self.insert_before_last_whitespace("]");
        }

        Ok(true)
    }

    fn parse_newline_delimited_json(&mut self) -> Result<()> {
        let mut initial = true;
        let mut processed_value = true;

        while processed_value {
            if !initial {
                let processed_comma = self.parse_character(',');
                if !processed_comma {
                    // repair: add missing comma
                    self.output = self.insert_before_last_whitespace(",");
                }
            } else {
                initial = false;
            }

            processed_value = self.parse_value()?;
        }

        if !processed_value {
            // repair: remove trailing comma
            self.output = self.strip_last_occurrence(",");
        }

        // repair: wrap the output inside array brackets
        self.output = format!("[\n{}\n]", self.output);
        Ok(())
    }

    fn parse_string(
        &mut self,
        stop_at_delimiter: bool,
        stop_at_index: Option<usize>,
    ) -> Result<bool> {
        let skip_escape_chars = self.current_char() == Some('\\');
        if skip_escape_chars {
            // repair: remove the first escape character
            self.i += 1;
        }

        if !self.is_quote(self.current_char()) {
            return Ok(false);
        }

        let quote_char = self.current_char().unwrap();
        let is_end_quote = self.get_end_quote_matcher(quote_char);

        let i_before = self.i;
        let o_before = self.output.len();

        let mut str_content = String::from("\"");
        self.i += 1;

        loop {
            if self.i >= self.chars.len() {
                // end of text, missing end quote
                let i_prev = self.prev_non_whitespace_index(self.i - 1);
                if !stop_at_delimiter && self.is_delimiter(self.chars.get(i_prev).copied()) {
                    // retry parsing the string, stopping at the first next delimiter
                    self.i = i_before;
                    self.output.truncate(o_before);
                    return self.parse_string(true, None);
                }

                // repair missing quote
                str_content = self.insert_before_last_whitespace_str(&str_content, "\"");
                self.output.push_str(&str_content);
                return Ok(true);
            }

            if let Some(stop_idx) = stop_at_index
                && self.i == stop_idx
            {
                str_content = self.insert_before_last_whitespace_str(&str_content, "\"");
                self.output.push_str(&str_content);
                return Ok(true);
            }

            let ch = self.current_char().unwrap();

            if is_end_quote(ch) {
                // end quote - verify if it's legit
                let i_quote = self.i;
                let o_quote = str_content.len();
                str_content.push('"');
                self.i += 1;
                self.output.push_str(&str_content);

                self.parse_whitespace_and_skip_comments(false);

                if stop_at_delimiter
                    || self.i >= self.chars.len()
                    || self.is_delimiter(self.current_char())
                    || self.is_quote(self.current_char())
                    || self.is_digit(self.current_char())
                {
                    // legitimate end quote
                    self.parse_concatenated_string()?;
                    return Ok(true);
                }

                let i_prev_char = self.prev_non_whitespace_index(i_quote - 1);
                let prev_char = self.chars.get(i_prev_char).copied();

                if prev_char == Some(',') {
                    // comma followed by quote - missing end quote before comma
                    self.i = i_before;
                    self.output.truncate(o_before);
                    return self.parse_string(false, Some(i_prev_char));
                }

                if self.is_delimiter(prev_char) {
                    // not the right end quote - retry with stop at delimiter
                    self.i = i_before;
                    self.output.truncate(o_before);
                    return self.parse_string(true, None);
                }

                // revert and continue - unescaped quote
                self.output.truncate(o_before);
                self.i = i_quote + 1;
                let prefix = str_content.get(..o_quote).unwrap_or("");
                str_content = format!("{}\\\"", prefix);
            } else if stop_at_delimiter && self.is_unquoted_string_delimiter(ch) {
                // stop at delimiter mode

                // test for URL like "https://..."
                let url_candidate = if self.chars.get(self.i - 1) == Some(&':')
                    && i_before + 1 < self.chars.len()
                    && self.i + 2 <= self.chars.len()
                {
                    self.chars.get(i_before + 1..self.i + 2)
                } else {
                    None
                };
                if url_candidate.is_some_and(|s| self.is_url_start(s)) {
                    while self.i < self.chars.len()
                        && self.is_url_char(self.current_char().unwrap())
                    {
                        str_content.push(self.current_char().unwrap());
                        self.i += 1;
                    }
                }

                // repair missing quote
                str_content = self.insert_before_last_whitespace_str(&str_content, "\"");
                self.output.push_str(&str_content);
                self.parse_concatenated_string()?;
                return Ok(true);
            } else if ch == '\\' {
                // handle escaped content
                if let Some(next_ch) = self.chars.get(self.i + 1) {
                    match next_ch {
                        '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' => {
                            let escaped: String = self
                                .chars
                                .get(self.i..self.i + 2)
                                .map(|s| s.iter().collect())
                                .unwrap_or_default();
                            str_content.push_str(&escaped);
                            self.i += 2;
                        }
                        ',' if skip_escape_chars => {
                            // Special case: escaped comma in escaped string should be treated as
                            // delimiter This creates a new array element
                            str_content =
                                self.insert_before_last_whitespace_str(&str_content, "\"");
                            self.output.push_str(&str_content);
                            self.output.push(',');
                            self.i += 2; // skip \,
                            self.parse_whitespace_and_skip_comments(true);
                            return Ok(true);
                        }
                        'u' => {
                            let mut j = 2;
                            while j < 6 && self.i + j < self.chars.len() {
                                let hex_ch = self.chars.get(self.i + j).copied().unwrap_or('\0');
                                if !self.is_hex(hex_ch) {
                                    break;
                                }
                                j += 1;
                            }

                            if j == 6 {
                                let unicode_seq: String = self
                                    .chars
                                    .get(self.i..self.i + 6)
                                    .map(|s| s.iter().collect())
                                    .unwrap_or_default();
                                str_content.push_str(&unicode_seq);
                                self.i += 6;
                            } else if self.i + j >= self.chars.len() {
                                // repair invalid unicode at end
                                self.i = self.chars.len();
                            } else {
                                // SAFETY: j <= 6 and self.i + j < self.chars.len() (else branch
                                // above handles >=)
                                let invalid_chars: String = self
                                    .chars
                                    .get(self.i..self.i + j.min(6))
                                    .map(|s| s.iter().collect())
                                    .unwrap_or_default();
                                return Err(JsonRepairError::InvalidUnicodeCharacter {
                                    chars: invalid_chars,
                                    position: self.i,
                                });
                            }
                        }
                        _ => {
                            // repair invalid escape character
                            str_content.push(*next_ch);
                            self.i += 2;
                        }
                    }
                } else {
                    self.i += 1;
                }
            } else {
                // handle regular characters
                if ch == '"' && self.chars.get(self.i - 1) != Some(&'\\') {
                    // repair unescaped double quote
                    str_content.push_str("\\\"");
                    self.i += 1;
                } else if self.is_control_character(ch) {
                    // unescaped control character
                    str_content.push_str(&self.get_control_character_escape(ch));
                    self.i += 1;
                } else {
                    if !self.is_valid_string_character(ch) {
                        return Err(JsonRepairError::InvalidCharacter {
                            character: ch,
                            position: self.i,
                        });
                    }
                    str_content.push(ch);
                    self.i += 1;
                }
            }

            if skip_escape_chars {
                // repair: skip escape character
                if self.current_char() == Some('\\') {
                    self.i += 1;
                }
            }
        }
    }

    fn parse_concatenated_string(&mut self) -> Result<bool> {
        let mut processed = false;

        self.parse_whitespace_and_skip_comments(true);
        while self.current_char() == Some('+') {
            processed = true;
            self.i += 1;
            self.parse_whitespace_and_skip_comments(true);

            // repair: remove the end quote of the first string
            self.output = self.strip_last_occurrence_with_remaining("\"", true);
            let start = self.output.len();
            let parsed_str = self.parse_string(false, None)?;
            if parsed_str {
                // repair: remove the start quote of the second string
                self.output = self.remove_at_index(start, 1);
            } else {
                // repair: remove the + because it's not followed by a string
                self.output = self.insert_before_last_whitespace("\"");
            }
        }

        Ok(processed)
    }

    fn parse_number(&mut self) -> Result<bool> {
        let start = self.i;

        if self.current_char() == Some('-') {
            self.i += 1;
            if self.at_end_of_number() {
                self.repair_number_ending_with_numeric_symbol(start);
                return Ok(true);
            }
            if !self.is_digit(self.current_char()) {
                self.i = start;
                return Ok(false);
            }
        }

        while self.is_digit(self.current_char()) {
            self.i += 1;
        }

        if self.current_char() == Some('.') {
            self.i += 1;
            if self.at_end_of_number() {
                self.repair_number_ending_with_numeric_symbol(start);
                return Ok(true);
            }
            if !self.is_digit(self.current_char()) {
                self.i = start;
                return Ok(false);
            }
            while self.is_digit(self.current_char()) {
                self.i += 1;
            }
        }

        if matches!(self.current_char(), Some('e' | 'E')) {
            self.i += 1;
            if matches!(self.current_char(), Some('-' | '+')) {
                self.i += 1;
            }
            if self.at_end_of_number() {
                self.repair_number_ending_with_numeric_symbol(start);
                return Ok(true);
            }
            if !self.is_digit(self.current_char()) {
                self.i = start;
                return Ok(false);
            }
            while self.is_digit(self.current_char()) {
                self.i += 1;
            }
        }

        if !self.at_end_of_number() {
            self.i = start;
            return Ok(false);
        }

        if self.i > start {
            let num: String = self
                .chars
                .get(start..self.i)
                .map(|s| s.iter().collect())
                .unwrap_or_default();
            let has_invalid_leading_zero =
                num.len() > 1 && num.starts_with('0') && self.is_digit(num.chars().nth(1));

            if has_invalid_leading_zero {
                self.output.push_str(&serde_json::to_string(&num).unwrap());
            } else {
                self.output.push_str(&num);
            }
            return Ok(true);
        }

        Ok(false)
    }

    fn parse_keywords(&mut self) -> bool {
        self.parse_keyword("true", "true")
            || self.parse_keyword("false", "false")
            || self.parse_keyword("null", "null")
            || self.parse_keyword("True", "true")
            || self.parse_keyword("False", "false")
            || self.parse_keyword("None", "null")
    }

    fn parse_keyword(&mut self, name: &str, value: &str) -> bool {
        let name_chars: Vec<char> = name.chars().collect();
        if self.i + name_chars.len() <= self.chars.len() {
            let slice = self.chars.get(self.i..self.i + name_chars.len());
            if slice == Some(name_chars.as_slice()) {
                self.output.push_str(value);
                self.i += name_chars.len();
                return true;
            }
        }
        false
    }

    fn parse_unquoted_string(&mut self, is_key: bool) -> Result<bool> {
        let start = self.i;

        if self.is_function_name_char_start(self.current_char()) {
            while self.i < self.chars.len() && self.is_function_name_char(self.current_char()) {
                self.i += 1;
            }

            let mut j = self.i;
            while j < self.chars.len() {
                let ch = self.chars.get(j).copied().unwrap_or('\0');
                if !self.is_whitespace(ch, true) {
                    break;
                }
                j += 1;
            }

            if j < self.chars.len() {
                let ch_at_j = self.chars.get(j).copied().unwrap_or('\0');
                if ch_at_j == '(' {
                    // function call
                    self.i = j + 1;
                    self.parse_value()?;
                    if self.current_char() == Some(')') {
                        self.i += 1;
                        if self.current_char() == Some(';') {
                            self.i += 1;
                        }
                    }
                    return Ok(true);
                }
            }
        }

        // reset for URL/unquoted string parsing
        self.i = start;

        while self.i < self.chars.len() {
            let ch = self.current_char().unwrap();
            if self.is_unquoted_string_delimiter(ch)
                || self.is_quote(Some(ch))
                || (is_key && ch == ':')
            {
                break;
            }
            self.i += 1;
        }

        // test for URL
        if self.i > 0
            && self.chars.get(self.i - 1) == Some(&':')
            && start < self.chars.len()
            && self.i + 2 <= self.chars.len()
            && self.is_url_start(self.chars.get(start..self.i + 2).unwrap_or(&[]))
        {
            while self.i < self.chars.len() && self.is_url_char(self.current_char().unwrap()) {
                self.i += 1;
            }
        }

        if self.i > start {
            // remove trailing whitespace
            while self.i > start
                && self.is_whitespace(self.chars.get(self.i - 1).copied().unwrap_or('\0'), true)
            {
                self.i -= 1;
            }

            let symbol: String = self
                .chars
                .get(start..self.i)
                .map(|s| s.iter().collect())
                .unwrap_or_default();

            if symbol == "undefined" {
                self.output.push_str("null");
            } else if is_key {
                // Keys must always be strings in JSON
                self.output
                    .push_str(&serde_json::to_string(&symbol).unwrap());
            } else {
                // Try to parse as number first, if that fails, treat as string
                if symbol.parse::<f64>().is_ok() {
                    // It's a valid number, output as-is
                    self.output.push_str(&symbol);
                } else {
                    // It's a string, so quote it using serde_json
                    self.output
                        .push_str(&serde_json::to_string(&symbol).unwrap());
                }
            }

            if self.current_char() == Some('"') {
                // missing start quote, skip end quote
                self.i += 1;
            }

            return Ok(true);
        }

        Ok(false)
    }

    fn parse_regex(&mut self) -> Result<bool> {
        if self.current_char() == Some('/') {
            let start = self.i;
            self.i += 1;

            while self.i < self.chars.len()
                && (self.current_char() != Some('/')
                    || (self.i > 0 && self.chars.get(self.i - 1) == Some(&'\\')))
            {
                self.i += 1;
            }

            // Only increment if we found the closing slash
            if self.i < self.chars.len() {
                self.i += 1;
            }

            let regex: String = self
                .chars
                .get(start..self.i)
                .map(|s| s.iter().collect())
                .unwrap_or_default();
            self.output
                .push_str(&serde_json::to_string(&regex).unwrap());
            return Ok(true);
        }
        Ok(false)
    }

    // Helper methods
    fn is_whitespace(&self, ch: char, skip_newline: bool) -> bool {
        if skip_newline {
            ch == ' ' || ch == '\n' || ch == '\t' || ch == '\r'
        } else {
            ch == ' ' || ch == '\t' || ch == '\r'
        }
    }

    fn is_special_whitespace(&self, ch: char) -> bool {
        matches!(ch as u32, 0xa0 | 0x2000..=0x200a | 0x202f | 0x205f | 0x3000)
    }

    fn is_quote(&self, ch: Option<char>) -> bool {
        matches!(
            ch,
            Some('"' | '\'' | '`' | '\u{2018}' | '\u{2019}' | '\u{201c}' | '\u{201d}' | '\u{00b4}')
        )
    }

    fn is_digit(&self, ch: Option<char>) -> bool {
        matches!(ch, Some('0'..='9'))
    }

    fn is_hex(&self, ch: char) -> bool {
        ch.is_ascii_hexdigit()
    }

    fn is_function_name_char_start(&self, ch: Option<char>) -> bool {
        matches!(ch, Some('a'..='z' | 'A'..='Z' | '_' | '$'))
    }

    fn is_function_name_char(&self, ch: Option<char>) -> bool {
        matches!(ch, Some('a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '$'))
    }

    fn is_delimiter(&self, ch: Option<char>) -> bool {
        matches!(
            ch,
            Some(',' | ':' | '[' | ']' | '/' | '{' | '}' | '(' | ')' | '\n' | '+')
        )
    }

    fn is_unquoted_string_delimiter(&self, ch: char) -> bool {
        matches!(ch, ',' | '[' | ']' | '/' | '{' | '}' | '\n' | '+')
    }

    fn is_start_of_value(&self, ch: Option<char>) -> bool {
        self.is_quote(ch)
            || matches!(
                ch,
                Some('[' | '{' | 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-')
            )
    }

    fn is_control_character(&self, ch: char) -> bool {
        matches!(ch, '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}')
    }

    fn is_valid_string_character(&self, ch: char) -> bool {
        ch >= '\u{0020}'
    }

    fn is_url_start(&self, chars: &[char]) -> bool {
        if chars.len() < 7 {
            return false;
        }
        let s: String = chars.iter().collect();
        s.starts_with("http://")
            || s.starts_with("https://")
            || s.starts_with("ftp://")
            || s.starts_with("mailto:")
            || s.starts_with("file://")
            || s.starts_with("data:")
            || s.starts_with("irc://")
    }

    fn is_url_char(&self, ch: char) -> bool {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '-' | '.'
                    | '_'
                    | '~'
                    | ':'
                    | '/'
                    | '?'
                    | '#'
                    | '@'
                    | '!'
                    | '$'
                    | '&'
                    | '\''
                    | '('
                    | ')'
                    | '*'
                    | '+'
                    | ';'
                    | '='
            )
    }

    fn get_end_quote_matcher(&self, quote_char: char) -> fn(char) -> bool {
        match quote_char {
            '"' => |ch| ch == '"',
            '\'' => |ch| ch == '\'',
            '\u{2018}' => |ch| matches!(ch, '\u{2018}' | '\u{2019}'),
            '\u{2019}' => |ch| matches!(ch, '\u{2018}' | '\u{2019}'),
            '\u{201c}' => |ch| matches!(ch, '\u{201c}' | '\u{201d}'),
            '\u{201d}' => |ch| matches!(ch, '\u{201c}' | '\u{201d}'),
            '`' => |ch| matches!(ch, '`' | '\u{00b4}' | '\''),
            '\u{00b4}' => |ch| matches!(ch, '`' | '\u{00b4}' | '\''),
            _ => |ch| ch == '"',
        }
    }

    fn get_control_character_escape(&self, ch: char) -> String {
        match ch {
            '\u{08}' => "\\b".to_string(),
            '\u{0c}' => "\\f".to_string(),
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            _ => ch.to_string(),
        }
    }

    fn at_end_of_block_comment(&self) -> bool {
        self.current_char() == Some('*') && self.chars.get(self.i + 1) == Some(&'/')
    }

    fn prev_non_whitespace_index(&self, start: usize) -> usize {
        let mut prev = start;
        while prev > 0
            && prev < self.chars.len()
            && self.is_whitespace(self.chars.get(prev).copied().unwrap_or('\0'), true)
        {
            prev = prev.saturating_sub(1);
        }
        prev
    }

    fn at_end_of_number(&self) -> bool {
        self.i >= self.chars.len()
            || self.is_delimiter(self.current_char())
            || self.is_whitespace(self.current_char().unwrap_or('\0'), true)
    }

    fn repair_number_ending_with_numeric_symbol(&mut self, start: usize) {
        let num: String = self
            .chars
            .get(start..self.i)
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        self.output.push_str(&format!("{num}0"));
    }

    fn ends_with_comma_or_newline(&self) -> bool {
        let regex = regex::Regex::new(r"[,\n][ \t\r]*$").unwrap();
        regex.is_match(&self.output)
    }

    fn strip_last_occurrence(&self, text_to_strip: &str) -> String {
        self.strip_last_occurrence_with_remaining(text_to_strip, false)
    }

    fn strip_last_occurrence_with_remaining(
        &self,
        text_to_strip: &str,
        strip_remaining: bool,
    ) -> String {
        if let Some(index) = self.output.rfind(text_to_strip) {
            let mut result = self.output.get(..index).unwrap_or("").to_string();
            if !strip_remaining {
                result.push_str(self.output.get(index + text_to_strip.len()..).unwrap_or(""));
            }
            result
        } else {
            self.output.clone()
        }
    }

    fn insert_before_last_whitespace(&self, text_to_insert: &str) -> String {
        let chars: Vec<char> = self.output.chars().collect();
        let mut index = chars.len();

        if index == 0 || !self.is_whitespace(chars.get(index - 1).copied().unwrap_or('\0'), true) {
            return format!("{}{}", self.output, text_to_insert);
        }

        while index > 0 && self.is_whitespace(chars.get(index - 1).copied().unwrap_or('\0'), true) {
            index -= 1;
        }

        // Convert the char-based index back to a byte offset for string slicing.
        let byte_index = self
            .output
            .char_indices()
            .nth(index)
            .map_or(self.output.len(), |(i, _)| i);

        format!(
            "{}{}{}",
            self.output.get(..byte_index).unwrap_or(""),
            text_to_insert,
            self.output.get(byte_index..).unwrap_or("")
        )
    }

    fn insert_before_last_whitespace_str(&self, text: &str, text_to_insert: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut index = chars.len();

        if index == 0 || !self.is_whitespace(chars.get(index - 1).copied().unwrap_or('\0'), true) {
            return format!("{text}{text_to_insert}");
        }

        while index > 0 && self.is_whitespace(chars.get(index - 1).copied().unwrap_or('\0'), true) {
            index -= 1;
        }

        // Convert the char-based index back to a byte offset for string slicing.
        let byte_index = text
            .char_indices()
            .nth(index)
            .map_or(text.len(), |(i, _)| i);

        format!(
            "{}{}{}",
            text.get(..byte_index).unwrap_or(""),
            text_to_insert,
            text.get(byte_index..).unwrap_or("")
        )
    }

    fn remove_at_index(&self, start: usize, count: usize) -> String {
        let chars: Vec<char> = self.output.chars().collect();
        let mut result = String::new();
        for (i, ch) in chars.iter().enumerate() {
            if i < start || i >= start + count {
                result.push(*ch);
            }
        }
        result
    }
}

pub fn json_repair<De: for<'de> Deserialize<'de>>(text: &str) -> Result<De> {
    let parser = JsonRepairParser::new(text.to_string());
    parser.parse()
}
