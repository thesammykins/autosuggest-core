//! Command-line tokenizer (`TECH.md §3.1`).
//!
//! Splits a raw input line plus a cursor byte-offset into [`Token`]s, tracking
//! each token's byte span, and identifies the token under (or just before) the
//! cursor — the "query" the user is currently typing.
//!
//! This module is **pure**: it performs no I/O. It is the first stage of the
//! `complete` pipeline (`tokenize -> parse -> complete -> rank`).
//!
//! # Quoting
//!
//! Single (`'`) and double (`"`) quotes group whitespace into one token. The
//! token's [`Token::text`] holds the *unquoted* value (what the program would
//! receive), while [`Token::raw`] preserves exactly what appears on the line
//! (including quote characters), so completion can decide whether to re-quote.
//!
//! # Cursor handling
//!
//! - If the cursor sits inside or at the trailing edge of a token, that token is
//!   the query and [`TokenList::query_prefix`] is the part *before* the cursor.
//! - If the cursor is after a separating space (e.g. `"ls "` with the cursor at
//!   the end), the query is empty and completion targets the *next* token.

/// A single token extracted from the input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The unquoted logical value (quotes removed, escapes resolved).
    pub text: String,
    /// The exact substring of the line this token came from (quotes included).
    pub raw: String,
    /// Byte offset of the token's first character in the line.
    pub start: usize,
    /// Byte offset one past the token's last character in the line.
    pub end: usize,
    /// Whether the token was quoted (single or double).
    pub quoted: bool,
}

impl Token {
    /// Byte span `(start, end)` of this token within the line.
    pub fn span(&self) -> (usize, usize) {
        (self.start, self.end)
    }
}

/// The result of tokenizing a line with a cursor: the tokens plus where the
/// cursor falls relative to them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenList {
    /// All tokens on the line, in order.
    pub tokens: Vec<Token>,
    /// Index of the token the cursor is editing, if the cursor is within or at
    /// the end of an existing token. `None` means the cursor is in fresh
    /// whitespace and a *new* (empty) token is being started.
    pub cursor_token: Option<usize>,
    /// The portion of the active token that lies *before* the cursor. This is
    /// the "query" used by ranking. Empty when starting a new token.
    pub query_prefix: String,
}

impl TokenList {
    /// The completion query: text of the active token up to the cursor.
    ///
    /// When the cursor is in fresh whitespace this is `""`, signalling that the
    /// next positional/subcommand/option slot should be completed wholesale.
    pub fn query(&self) -> &str {
        &self.query_prefix
    }

    /// Tokens that appear strictly before the cursor position and are therefore
    /// already "committed" input the parser must consume. When the cursor edits
    /// an existing token, that token is excluded (it is the in-progress query);
    /// when the cursor is in whitespace, all tokens are committed.
    pub fn committed(&self) -> &[Token] {
        match self.cursor_token {
            Some(i) => &self.tokens[..i],
            None => &self.tokens,
        }
    }
}

/// Tokenize `line`, locating the token under `cursor` (a byte offset).
///
/// `cursor` is clamped to `0..=line.len()` and snapped to the nearest char
/// boundary so callers cannot trigger a panic with an interior-byte offset.
pub fn tokenize(line: &str, cursor: usize) -> TokenList {
    let cursor = clamp_cursor(line, cursor);
    let tokens = lex(line);

    // Find the token the cursor edits: the cursor must fall within the token's
    // span. A cursor exactly at `token.end` counts as editing that token only
    // when no whitespace separates it from the cursor (handled because the
    // lexer's `end` is the char after the last token byte; a following space
    // would place the cursor past `end`).
    let mut cursor_token = None;
    for (i, tok) in tokens.iter().enumerate() {
        if cursor >= tok.start && cursor <= tok.end {
            // Prefer editing this token unless the cursor is exactly at `end`
            // and the next byte is whitespace / end-of-line (i.e. a new token).
            if cursor < tok.end {
                cursor_token = Some(i);
                break;
            }
            // cursor == tok.end: editing iff there is no gap before it, which is
            // always true here, but we must ensure the cursor is not actually in
            // the whitespace that follows. Since `end` is exclusive of trailing
            // space, cursor == end with a trailing space means "new token".
            let next_is_boundary = line[cursor..]
                .chars()
                .next()
                .map(char::is_whitespace)
                .unwrap_or(true);
            if next_is_boundary && cursor == line.len() {
                // At end of line with no trailing space => still editing token.
                cursor_token = Some(i);
                break;
            }
            if !next_is_boundary {
                cursor_token = Some(i);
                break;
            }
            // Otherwise the cursor is at the edge before whitespace: new token.
        }
    }

    let query_prefix = match cursor_token {
        Some(i) => {
            let tok = &tokens[i];
            // The query is the unquoted text up to the cursor. We recompute it
            // from the raw slice so multi-byte/quoted prefixes are honoured.
            unquote(&line[tok.start..cursor]).0
        }
        None => String::new(),
    };

    TokenList {
        tokens,
        cursor_token,
        query_prefix,
    }
}

/// Clamp `cursor` into the line and snap to a char boundary (round down).
fn clamp_cursor(line: &str, cursor: usize) -> usize {
    let mut c = cursor.min(line.len());
    while c > 0 && !line.is_char_boundary(c) {
        c -= 1;
    }
    c
}

/// Lex the line into tokens, honouring single/double quotes and backslash
/// escapes. Whitespace separates tokens; runs of whitespace are skipped.
fn lex(line: &str) -> Vec<Token> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        // Skip leading whitespace.
        while i < len && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }

        let start = i;
        let mut text = String::new();
        let mut quoted = false;

        while i < len {
            let ch = bytes[i] as char;
            if ch.is_whitespace() {
                break;
            }
            match ch {
                '\'' => {
                    quoted = true;
                    i += 1;
                    while i < len && bytes[i] != b'\'' {
                        // Single quotes are literal: no escape processing.
                        let c = next_char(line, i);
                        text.push(c);
                        i += c.len_utf8();
                    }
                    if i < len {
                        i += 1; // closing quote
                    }
                }
                '"' => {
                    quoted = true;
                    i += 1;
                    while i < len && bytes[i] != b'"' {
                        if bytes[i] == b'\\' && i + 1 < len {
                            // In double quotes, backslash escapes the next char.
                            i += 1;
                            let c = next_char(line, i);
                            text.push(c);
                            i += c.len_utf8();
                        } else {
                            let c = next_char(line, i);
                            text.push(c);
                            i += c.len_utf8();
                        }
                    }
                    if i < len {
                        i += 1; // closing quote
                    }
                }
                '\\' if i + 1 < len => {
                    // Unquoted escape: take the next char literally.
                    i += 1;
                    let c = next_char(line, i);
                    text.push(c);
                    i += c.len_utf8();
                }
                _ => {
                    let c = next_char(line, i);
                    text.push(c);
                    i += c.len_utf8();
                }
            }
        }

        let end = i;
        tokens.push(Token {
            text,
            raw: line[start..end].to_string(),
            start,
            end,
            quoted,
        });
    }

    tokens
}

/// Decode the char starting at byte index `i` (assumed a char boundary).
fn next_char(line: &str, i: usize) -> char {
    line[i..].chars().next().unwrap_or('\u{FFFD}')
}

/// Strip surrounding/embedded quotes from a raw slice, returning the logical
/// text and whether any quoting was present. Used to compute the query prefix.
fn unquote(raw: &str) -> (String, bool) {
    let mut out = String::new();
    let mut quoted = false;
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                quoted = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    out.push(c);
                }
            }
            '"' => {
                quoted = true;
                while let Some(c) = chars.next() {
                    if c == '"' {
                        break;
                    }
                    if c == '\\' {
                        if let Some(escaped) = chars.next() {
                            out.push(escaped);
                        }
                    } else {
                        out.push(c);
                    }
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            _ => out.push(ch),
        }
    }
    (out, quoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_line_has_no_tokens() {
        let t = tokenize("", 0);
        assert!(t.tokens.is_empty());
        assert_eq!(t.cursor_token, None);
        assert_eq!(t.query(), "");
    }

    #[test]
    fn single_token_cursor_at_end_is_editing() {
        let t = tokenize("git", 3);
        assert_eq!(t.tokens.len(), 1);
        assert_eq!(t.tokens[0].text, "git");
        assert_eq!(t.tokens[0].span(), (0, 3));
        assert_eq!(t.cursor_token, Some(0));
        assert_eq!(t.query(), "git");
        assert_eq!(t.committed().len(), 0);
    }

    #[test]
    fn cursor_in_middle_of_token() {
        let t = tokenize("status", 2);
        assert_eq!(t.cursor_token, Some(0));
        assert_eq!(t.query(), "st");
    }

    #[test]
    fn trailing_space_starts_new_token() {
        let t = tokenize("ls ", 3);
        assert_eq!(t.tokens.len(), 1);
        assert_eq!(t.cursor_token, None);
        assert_eq!(t.query(), "");
        assert_eq!(t.committed().len(), 1);
    }

    #[test]
    fn two_tokens_cursor_on_second() {
        let t = tokenize("git ch", 6);
        assert_eq!(t.tokens.len(), 2);
        assert_eq!(t.cursor_token, Some(1));
        assert_eq!(t.query(), "ch");
        assert_eq!(t.committed().len(), 1);
        assert_eq!(t.committed()[0].text, "git");
    }

    #[test]
    fn double_quoted_arg_groups_whitespace() {
        let t = tokenize("echo \"a b\"", 10);
        assert_eq!(t.tokens.len(), 2);
        assert_eq!(t.tokens[1].text, "a b");
        assert!(t.tokens[1].quoted);
        assert_eq!(t.tokens[1].raw, "\"a b\"");
    }

    #[test]
    fn single_quoted_arg_is_literal() {
        let t = tokenize("echo 'x\\y'", 10);
        assert_eq!(t.tokens[1].text, "x\\y");
        assert!(t.tokens[1].quoted);
    }

    #[test]
    fn cursor_inside_quoted_prefix() {
        // Cursor right after the opening quote and one char.
        let line = "cd \"My ";
        let t = tokenize(line, line.len());
        assert_eq!(t.cursor_token, Some(1));
        assert_eq!(t.query(), "My ");
    }

    #[test]
    fn cursor_clamped_and_snapped() {
        // Multi-byte char; an interior cursor must not panic.
        let line = "café";
        let t = tokenize(line, 4); // interior of 'é' (é is 2 bytes at 3..5)
        assert_eq!(t.tokens.len(), 1);
        // cursor snapped down to byte 3 => query "caf"
        assert_eq!(t.query(), "caf");
    }

    #[test]
    fn cursor_beyond_end_clamped() {
        let t = tokenize("ls", 999);
        assert_eq!(t.cursor_token, Some(0));
        assert_eq!(t.query(), "ls");
    }

    #[test]
    fn multiple_spaces_between_tokens() {
        let t = tokenize("git    status", 13);
        assert_eq!(t.tokens.len(), 2);
        assert_eq!(t.tokens[1].text, "status");
        assert_eq!(t.tokens[1].start, 7);
    }
}
