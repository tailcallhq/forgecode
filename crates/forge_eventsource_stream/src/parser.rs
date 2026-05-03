use nom::branch::alt;
use nom::bytes::complete::take_while_m_n as take_while_m_n_complete;
use nom::bytes::streaming::{tag, take_while, take_while_m_n, take_while1};
use nom::combinator::opt;
use nom::sequence::{preceded, terminated};
use nom::{IResult, Parser};

/// ; ABNF definition from HTML spec
///
/// stream        = [ bom ] *event
/// event         = *( comment / field ) end-of-line
/// comment       = colon *any-char end-of-line
/// field         = 1*name-char [ colon [ space ] *any-char ] end-of-line
/// end-of-line   = ( cr lf / cr / lf )
///
/// ; characters
/// lf            = %x000A ; U+000A LINE FEED (LF)
/// cr            = %x000D ; U+000D CARRIAGE RETURN (CR)
/// space         = %x0020 ; U+0020 SPACE
/// colon         = %x003A ; U+003A COLON (:)
/// bom           = %xFEFF ; U+FEFF BYTE ORDER MARK
/// name-char     = %x0000-0009 / %x000B-000C / %x000E-0039 / %x003B-10FFFF
///                 ; a scalar value other than U+000A LINE FEED (LF), U+000D
/// CARRIAGE RETURN (CR), or U+003A COLON (:) any-char      = %x0000-0009 /
/// %x000B-000C / %x000E-10FFFF                 ; a scalar value other than
/// U+000A LINE FEED (LF) or U+000D CARRIAGE RETURN (CR)

#[derive(Debug)]
#[allow(dead_code)]
pub enum RawEventLine<'a> {
    Comment(&'a str),
    Field(&'a str, Option<&'a str>),
    Empty,
}

#[inline]
pub fn is_lf(c: char) -> bool {
    c == '\u{000A}'
}

#[inline]
pub fn is_cr(c: char) -> bool {
    c == '\u{000D}'
}

#[inline]
pub fn is_space(c: char) -> bool {
    c == '\u{0020}'
}

#[inline]
pub fn is_colon(c: char) -> bool {
    c == '\u{003A}'
}

#[inline]
pub fn is_bom(c: char) -> bool {
    c == '\u{feff}'
}

#[inline]
pub fn is_name_char(c: char) -> bool {
    matches!(c, '\u{0000}'..='\u{0009}'
        | '\u{000B}'..='\u{000C}'
        | '\u{000E}'..='\u{0039}'
        | '\u{003B}'..='\u{10FFFF}')
}

#[inline]
pub fn is_any_char(c: char) -> bool {
    matches!(c, '\u{0000}'..='\u{0009}'
        | '\u{000B}'..='\u{000C}'
        | '\u{000E}'..='\u{10FFFF}')
}

#[inline]
fn crlf(input: &str) -> IResult<&str, &str> {
    tag("\u{000D}\u{000A}").parse(input)
}

#[inline]
fn end_of_line(input: &str) -> IResult<&str, &str> {
    alt((
        crlf,
        take_while_m_n_complete(1, 1, is_cr),
        take_while_m_n_complete(1, 1, is_lf),
    ))
    .parse(input)
}

#[inline]
fn comment(input: &str) -> IResult<&str, RawEventLine<'_>> {
    preceded(
        take_while_m_n(1, 1, is_colon),
        terminated(take_while(is_any_char), end_of_line),
    )
    .parse(input)
    .map(|(input, comment)| (input, RawEventLine::Comment(comment)))
}

#[inline]
fn field(input: &str) -> IResult<&str, RawEventLine<'_>> {
    terminated(
        (
            take_while1(is_name_char),
            opt(preceded(
                take_while_m_n(1, 1, is_colon),
                preceded(opt(take_while_m_n(1, 1, is_space)), take_while(is_any_char)),
            )),
        ),
        end_of_line,
    )
    .parse(input)
    .map(|(input, (field, data))| (input, RawEventLine::Field(field, data)))
}

#[inline]
fn empty(input: &str) -> IResult<&str, RawEventLine<'_>> {
    end_of_line(input).map(|(i, _)| (i, RawEventLine::Empty))
}

pub fn line(input: &str) -> IResult<&str, RawEventLine<'_>> {
    alt((comment, field, empty)).parse(input)
}
