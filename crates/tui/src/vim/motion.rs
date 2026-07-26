//! Pure cursor-movement helpers for the vim engine.
//!
//! Every function here takes `&str` + a char index and returns a NEW char
//! index; none mutate. Movement treats the buffer as a sequence of logical
//! lines separated by `\n` and a sequence of *words* classified by char kind
//! (`Word` = alphanumeric/`_`, `Punct` = other non-whitespace, `Space` =
//! whitespace incl. `\n`).

pub(crate) fn byte_offset_for_char(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

pub(crate) fn char_index_at_byte(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

/// Index of the first char of the logical line containing `cursor`.
pub fn line_start(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let c = cursor.min(chars.len());
    (0..c)
        .rev()
        .find(|&i| chars[i] == '\n')
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// Index just past the last non-newline char of the current line (i.e. the
/// index of the `\n` ending the line, or the char-length of the text if this
/// is the last line). EXCLUSIVE end.
pub fn line_end_exclusive(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let start = line_start(text, cursor);
    (start..chars.len())
        .find(|&i| chars[i] == '\n')
        .unwrap_or(chars.len())
}

/// Index of the first non-whitespace char on the current line (or line_end if
/// the line is entirely blank).
pub fn line_first_nonblank(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let start = line_start(text, cursor);
    let end = line_end_exclusive(text, cursor);
    (start..end)
        .find(|&i| !chars[i].is_whitespace())
        .unwrap_or(end)
}

/// Char index of the start of the Nth logical line (1-indexed). Clamps to the
/// last line when out of range.
pub fn line_start_by_number(text: &str, one_based: usize) -> usize {
    if one_based == 0 || one_based == 1 {
        return 0;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut line = 1usize;
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '\n' {
            line += 1;
            if line == one_based {
                return i + 1;
            }
        }
    }
    // Clamped to last line start.
    line_start(text, chars.len())
}

/// Total number of logical lines (a non-empty text with no trailing newline
/// counts as 1; each `\n` introduces a new line).
pub fn line_count(text: &str) -> usize {
    if text.is_empty() {
        return 1;
    }
    text.chars().filter(|&c| c == '\n').count() + 1
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Word,
    Punct,
    Space,
}

fn classify(ch: char) -> Kind {
    if ch.is_whitespace() {
        Kind::Space
    } else if ch.is_alphanumeric() || ch == '_' {
        Kind::Word
    } else {
        Kind::Punct
    }
}

/// vim `w`: position of the start of the next word. A "word" is a maximal run
/// of one non-Space class; `w` skips the rest of the current word and any
/// intervening whitespace.
pub fn word_forward(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let mut i = cursor.min(chars.len());
    if i >= chars.len() {
        return chars.len().saturating_sub(1);
    }
    let kind = classify(chars[i]);
    // Skip the rest of the current word (same class, non-Space).
    if kind != Kind::Space {
        while i < chars.len() && classify(chars[i]) == kind {
            i += 1;
        }
    }
    // Skip whitespace.
    while i < chars.len() && classify(chars[i]) == Kind::Space {
        i += 1;
    }
    if i >= chars.len() {
        // Landed on EOF; vim clamps to last char.
        chars.len().saturating_sub(1)
    } else {
        i
    }
}

/// vim `b`: position of the start of the previous word.
pub fn word_backward(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let mut i = cursor.min(chars.len());
    if i == 0 {
        return 0;
    }
    i -= 1;
    // Skip preceding whitespace.
    while i > 0 && classify(chars[i]) == Kind::Space {
        i -= 1;
    }
    if classify(chars[i]) == Kind::Space {
        return i;
    }
    let kind = classify(chars[i]);
    // Walk back to the start of this word.
    while i > 0 && classify(chars[i - 1]) == kind {
        i -= 1;
    }
    i
}

/// vim `e`: position of the last char of the current word if the cursor is not
/// already on its last char, else the last char of the next word (skipping
/// whitespace).
pub fn word_end(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let mut i = cursor.min(chars.len());
    if i >= chars.len() {
        i = chars.len() - 1;
    }
    // If on whitespace or at the last char of a word, advance to next word first.
    let on_space = classify(chars[i]) == Kind::Space;
    let at_word_end = i + 1 >= chars.len() || classify(chars[i + 1]) != classify(chars[i]);
    if on_space || at_word_end {
        i += 1;
        while i < chars.len() && classify(chars[i]) == Kind::Space {
            i += 1;
        }
    }
    if i >= chars.len() {
        return chars.len() - 1;
    }
    let kind = classify(chars[i]);
    if kind == Kind::Space {
        return i;
    }
    while i + 1 < chars.len() && classify(chars[i + 1]) == kind {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_helpers_single_line() {
        let s = "hello world";
        assert_eq!(line_start(s, 0), 0);
        assert_eq!(line_start(s, 5), 0);
        assert_eq!(line_end_exclusive(s, 0), 11);
        assert_eq!(line_end_exclusive(s, 10), 11);
        assert_eq!(line_first_nonblank(s, 0), 0);
        assert_eq!(line_count(s), 1);
    }

    #[test]
    fn line_helpers_multiline() {
        let s = "foo\nbar\nbaz";
        assert_eq!(line_start(s, 0), 0);
        assert_eq!(line_start(s, 4), 4); // start of "bar"
        assert_eq!(line_start(s, 5), 4);
        assert_eq!(line_end_exclusive(s, 0), 3); // at '\n' index 3
        assert_eq!(line_end_exclusive(s, 4), 7);
        assert_eq!(line_end_exclusive(s, 8), 11);
        assert_eq!(line_count(s), 3);
        assert_eq!(line_start_by_number(s, 2), 4);
        assert_eq!(line_start_by_number(s, 3), 8);
        assert_eq!(line_start_by_number(s, 99), 8); // clamp
        assert_eq!(line_first_nonblank(s, 4), 4);
    }

    #[test]
    fn line_helpers_blank_line() {
        let s = "foo\n\nbaz";
        assert_eq!(line_start(s, 4), 4); // the blank middle line content start
        assert_eq!(line_end_exclusive(s, 4), 4); // '\n' at 5? no: idx4 is '\n'
                                                 // cursor=4 sits on the '\n' terminating the blank line; its line content
                                                 // range is empty [4,4) so first_nonblank returns line_end (4).
        assert_eq!(line_first_nonblank(s, 4), 4);
        assert_eq!(line_count(s), 3);
    }

    #[test]
    fn word_forward_simple() {
        let s = "foo bar baz";
        assert_eq!(word_forward(s, 0), 4); // foo -> bar
        assert_eq!(word_forward(s, 4), 8); // bar -> baz
        assert_eq!(word_forward(s, 8), 10); // baz -> last char (clamp)
    }

    #[test]
    fn word_forward_punct_words() {
        let s = "foo.bar baz";
        // foo (Word) .bar (Punct, then Word? no: '.' is Punct, 'bar' is Word)
        assert_eq!(word_forward(s, 0), 3); // foo -> '.'
        assert_eq!(word_forward(s, 3), 4); // '.' -> bar
        assert_eq!(word_forward(s, 4), 8); // bar -> baz
    }

    #[test]
    fn word_forward_multiline_and_whitespace() {
        let s = "foo   \nbar";
        // from start of foo, skip foo, skip spaces+newline, land on bar (idx 7)
        assert_eq!(word_forward(s, 0), 7);
        let s2 = "a  b";
        assert_eq!(word_forward(s2, 0), 3); // skip a, two spaces, land on b
    }

    #[test]
    fn word_backward_simple() {
        let s = "foo bar baz";
        assert_eq!(word_backward(s, 11), 8); // baz -> bar
        assert_eq!(word_backward(s, 8), 4); // bar -> foo
        assert_eq!(word_backward(s, 4), 0);
        assert_eq!(word_backward(s, 0), 0);
    }

    #[test]
    fn word_backward_punct() {
        let s = "foo.bar";
        assert_eq!(word_backward(s, 7), 4); // -> bar
        assert_eq!(word_backward(s, 4), 3); // -> '.'
        assert_eq!(word_backward(s, 3), 0); // -> foo
    }

    #[test]
    fn word_end_basic() {
        let s = "foo bar baz";
        assert_eq!(word_end(s, 0), 2); // last of foo
        assert_eq!(word_end(s, 2), 6); // -> last of bar
        assert_eq!(word_end(s, 4), 6); // mid bar -> last of bar
        assert_eq!(word_end(s, 6), 10); // -> last of baz
    }

    #[test]
    fn word_end_punct() {
        let s = "a.bc";
        assert_eq!(word_end(s, 0), 1); // a -> '.' (Punct word)
        assert_eq!(word_end(s, 1), 3); // '.' -> last of bc
    }

    #[test]
    fn word_motions_empty_and_boundaries() {
        assert_eq!(word_forward("", 0), 0);
        assert_eq!(word_backward("", 0), 0);
        assert_eq!(word_end("", 0), 0);
        let s = "ab";
        assert_eq!(word_forward(s, 0), 1); // clamps to last char
        assert_eq!(word_forward(s, 1), 1);
        assert_eq!(word_end(s, 0), 1);
    }

    #[test]
    fn byte_char_conversion_roundtrip() {
        let s = "héllo"; // 'é' is two bytes
        assert_eq!(byte_offset_for_char(s, 0), 0);
        assert_eq!(byte_offset_for_char(s, 1), 1);
        assert_eq!(byte_offset_for_char(s, 2), 3); // past é
        assert_eq!(char_index_at_byte(s, 3), 2);
        assert_eq!(char_index_at_byte(s, 0), 0);
    }
}
