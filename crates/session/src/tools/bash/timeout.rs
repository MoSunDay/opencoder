//! Pure command-text timeout resolution for the bash tool.

pub(super) const MIN_TIMEOUT_SECS: u64 = 30;
pub(super) const MAX_TIMEOUT_SECS: u64 = 600;
const SLEEP_PADDING_SECS: u64 = 120;
const LEGACY_PROD_MIN_TIMEOUT_SECS: u64 = 120;

#[cfg(not(test))]
const LEGACY_MIN_TIMEOUT_SECS: u64 = LEGACY_PROD_MIN_TIMEOUT_SECS;
#[cfg(test)]
const LEGACY_MIN_TIMEOUT_SECS: u64 = 1;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ResolvedTimeout<'a> {
    pub command: &'a str,
    pub timeout_secs: u64,
    pub display_secs: u64,
}

/// Resolve the foreground deadline while preserving the established silent
/// `timeout N; command` prefix contract. The prefix is stripped before
/// execution; otherwise only unquoted command-position `timeout N`/`sleep N`
/// invocations influence the deadline and the original command is retained.
pub(super) fn resolve(
    command: &str,
    default_timeout_secs: u64,
    default_display_secs: u64,
) -> ResolvedTimeout<'_> {
    if let Some((raw, rest)) = parse_legacy_prefix(command) {
        let secs = raw.clamp(LEGACY_MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
        return ResolvedTimeout {
            command: rest,
            timeout_secs: secs,
            display_secs: secs,
        };
    }

    let (timeout_hint, sleep_hint) = command_hints(command);
    if let Some(raw) = timeout_hint {
        let secs = raw.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
        return ResolvedTimeout {
            command,
            timeout_secs: secs,
            display_secs: secs,
        };
    }
    if let Some(raw) = sleep_hint {
        let secs = SLEEP_PADDING_SECS + raw.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
        return ResolvedTimeout {
            command,
            timeout_secs: secs,
            display_secs: secs,
        };
    }

    ResolvedTimeout {
        command,
        timeout_secs: default_timeout_secs,
        display_secs: default_display_secs,
    }
}

/// Parse the historical silent override prefix: `timeout N; command`.
fn parse_legacy_prefix(command: &str) -> Option<(u64, &str)> {
    let rest = command.strip_prefix("timeout")?;
    let bytes = rest.as_bytes();
    let mut cursor = 0;
    let ws_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if cursor == ws_start {
        return None;
    }
    let digits_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == digits_start {
        return None;
    }
    let raw = parse_u64_saturating(&bytes[digits_start..cursor]);
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if raw == 0 || bytes.get(cursor) != Some(&b';') {
        return None;
    }
    Some((raw, &rest[cursor + 1..]))
}

/// Find integer hints only where the keyword is the command word of a shell
/// statement. Quoted strings, comments and ordinary arguments are ignored so
/// log text such as `echo 'sleep 600'` cannot extend foreground occupancy.
fn command_hints(command: &str) -> (Option<u64>, Option<u64>) {
    let bytes = command.as_bytes();
    let mut timeout_max = None;
    let mut sleep_max = None;
    let mut cursor = 0;
    let mut command_position = true;

    while cursor < bytes.len() {
        match bytes[cursor] {
            b' ' | b'\t' | b'\r' => cursor += 1,
            b'\n' | b';' | b'|' | b'&' | b'(' | b'{' => {
                command_position = true;
                cursor += 1;
            }
            b')' | b'}' => {
                // Closing group delimiters end the current command but do not
                // themselves start another one. They must still advance the
                // scanner: `skip_word` deliberately stops at delimiters.
                command_position = false;
                cursor += 1;
            }
            b'#' if command_position => {
                cursor = bytes[cursor..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(bytes.len(), |offset| cursor + offset);
            }
            b'\'' | b'"' => {
                cursor = skip_quoted(bytes, cursor, bytes[cursor]);
                command_position = false;
            }
            _ => {
                let start = cursor;
                cursor = skip_word(bytes, cursor);
                if !command_position {
                    continue;
                }
                let word = &bytes[start..cursor];
                let hint = parse_following_integer(bytes, cursor);
                if word == b"timeout" {
                    if let Some(value) = hint {
                        timeout_max = Some(timeout_max.map_or(value, |old: u64| old.max(value)));
                    }
                } else if word == b"sleep" {
                    if let Some(value) = hint {
                        sleep_max = Some(sleep_max.map_or(value, |old: u64| old.max(value)));
                    }
                }
                command_position = false;
            }
        }
    }

    (timeout_max, sleep_max)
}

fn skip_word(bytes: &[u8], mut cursor: usize) -> usize {
    while let Some(byte) = bytes.get(cursor) {
        if byte.is_ascii_whitespace() || b";|&(){}".contains(byte) {
            break;
        }
        if *byte == b'\\' && cursor + 1 < bytes.len() {
            cursor += 2;
        } else if matches!(*byte, b'\'' | b'"') {
            cursor = skip_quoted(bytes, cursor, *byte);
        } else {
            cursor += 1;
        }
    }
    cursor
}

fn skip_quoted(bytes: &[u8], mut cursor: usize, quote: u8) -> usize {
    cursor += 1;
    while cursor < bytes.len() {
        if bytes[cursor] == quote {
            return cursor + 1;
        }
        if quote == b'"' && bytes[cursor] == b'\\' && cursor + 1 < bytes.len() {
            cursor += 2;
        } else {
            cursor += 1;
        }
    }
    cursor
}

fn parse_following_integer(bytes: &[u8], mut cursor: usize) -> Option<u64> {
    if !bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    let start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == start || !has_integer_boundary(bytes, cursor) {
        return None;
    }
    Some(parse_u64_saturating(&bytes[start..cursor]))
}

fn parse_u64_saturating(digits: &[u8]) -> u64 {
    digits.iter().fold(0u64, |value, digit| {
        value
            .saturating_mul(10)
            .saturating_add(u64::from(digit - b'0'))
    })
}

fn has_integer_boundary(bytes: &[u8], cursor: usize) -> bool {
    cursor == bytes.len()
        || (!bytes[cursor].is_ascii_alphanumeric()
            && bytes[cursor] != b'_'
            && bytes[cursor] != b'.')
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_TIMEOUT: u64 = 130;
    const DEFAULT_DISPLAY: u64 = 120;

    fn resolved(command: &str) -> ResolvedTimeout<'_> {
        resolve(command, DEFAULT_TIMEOUT, DEFAULT_DISPLAY)
    }

    #[test]
    fn legacy_prefix_is_stripped_and_uses_compatible_test_floor() {
        assert_eq!(LEGACY_PROD_MIN_TIMEOUT_SECS, 120);
        assert_eq!(
            resolved("timeout 2; sleep 5"),
            ResolvedTimeout {
                command: " sleep 5",
                timeout_secs: 2,
                display_secs: 2,
            }
        );
        assert_eq!(resolved("timeout 7;").command, "");
    }

    #[test]
    fn real_timeout_is_clamped_and_command_is_retained() {
        assert_eq!(resolved("timeout 10 cargo test").timeout_secs, 30);
        assert_eq!(resolved("echo begin; timeout 300 task").timeout_secs, 300);
        assert_eq!(resolved("timeout 900 task").timeout_secs, 600);
        assert_eq!(resolved("timeout 45 task").command, "timeout 45 task");
    }

    #[test]
    fn largest_command_position_hint_wins() {
        assert_eq!(resolved("timeout 45 a; timeout 240 b").timeout_secs, 240);
        assert_eq!(resolved("sleep 30; sleep 90").timeout_secs, 210);
    }

    #[test]
    fn sleep_clamps_x_then_adds_the_full_padding() {
        assert_eq!(resolved("sleep 0").timeout_secs, 150);
        assert_eq!(resolved("sleep 500").timeout_secs, 620);
        assert_eq!(resolved("sleep 600").timeout_secs, 720);
        assert_eq!(resolved("sleep 700").timeout_secs, 720);
    }

    #[test]
    fn explicit_timeout_wins_over_sleep() {
        assert_eq!(resolved("sleep 500; timeout 45 command").timeout_secs, 45);
    }

    #[test]
    fn quotes_comments_and_arguments_do_not_extend_timeout() {
        for command in [
            "echo 'sleep 500'",
            "echo \"timeout 500\"",
            "echo sleep 500",
            "echo ok # timeout 500",
        ] {
            assert_eq!(resolved(command).timeout_secs, DEFAULT_TIMEOUT, "{command}");
        }
    }

    #[test]
    fn grouped_commands_terminate_the_scan() {
        assert_eq!(
            resolved("(echo hi 2>&1) | head").timeout_secs,
            DEFAULT_TIMEOUT
        );
        assert_eq!(resolved("{ sleep 40; }").timeout_secs, 160);
    }

    #[test]
    fn invalid_or_non_integer_hints_are_ignored() {
        for command in [
            "timeout command",
            "timeout 5m command",
            "sleep 0.2",
            "sleep 5s",
        ] {
            assert_eq!(resolved(command).timeout_secs, DEFAULT_TIMEOUT, "{command}");
        }
    }

    #[test]
    fn huge_values_saturate_then_cap() {
        assert_eq!(
            resolved("sleep 999999999999999999999999999").timeout_secs,
            720
        );
    }
}
