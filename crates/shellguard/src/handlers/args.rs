//! Argument-inspection helpers shared by handlers.
//!
//! Ported verbatim from rippy's `handlers/mod.rs` (MIT,
//! https://github.com/mpecan/rippy).

/// Helper: check if any arg matches a set of flags.
pub(crate) fn has_flag(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|a| flags.contains(&a.as_str()))
}

/// Helper: true only when a help/version flag is the command's SOLE argument.
///
/// SECURITY: help/version flags must NOT be matched anywhere in argv. Many
/// commands overload short flags (`docker -h` is `--hostname`, not `--help`) or
/// consume the next token as a value (`git commit -m --version`), so scanning
/// argv for a help flag and short-circuiting to Allow lets a dangerous operand
/// ride along auto-approved. A lone help/version flag is genuinely inert
/// everywhere; combined with any other argument it must never pre-empt
/// evaluation of the rest of the command.
pub(crate) fn is_sole_help_flag(args: &[String], flags: &[&str]) -> bool {
    args.len() == 1 && flags.contains(&args[0].as_str())
}

/// Helper: get the first positional argument (non-flag).
pub(crate) fn first_positional(args: &[String]) -> Option<&str> {
    args.iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str)
}

/// Helper: collect all positional (non-flag) arguments.
pub(crate) fn positional_args(args: &[String]) -> Vec<&str> {
    args.iter()
        .filter(|a| !a.starts_with('-'))
        .map(String::as_str)
        .collect()
}

/// Helper: get the value following a flag (e.g., `-o output.txt` -> `Some("output.txt")`).
pub(crate) fn get_flag_value(args: &[String], flags: &[&str]) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        if flags.contains(&arg.as_str()) {
            return args.get(i + 1).cloned();
        }
    }
    None
}

/// Helper: check if any arg matches a flag exactly OR as its `flag=value` form.
///
/// `has_flag`/`get_flag_value` only match space-separated tokens, so
/// `--use-compress-program=sh` or `--output=/tmp/x` slip past them. This
/// catches both forms without matching an unrelated longer flag name
/// (`--foo=bar` matches `--foo`, not `--foobar`).
pub(crate) fn has_flag_or_prefixed(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|a| {
        flags
            .iter()
            .any(|f| a == f || a.strip_prefix(f).is_some_and(|rest| rest.starts_with('=')))
    })
}

/// Helper: check if any arg matches a short (single-dash, two-char) flag glued
/// directly to its value with no separator (getopt's `-Ivalue`, e.g. tar's
/// `-Ish` for `--use-compress-program=sh`).
///
/// `has_flag_or_prefixed` only catches the `flag=value` form, so a glued short
/// option slips past it. This is restricted to two-char flags (`-I`, `-F`) --
/// long options never take a glued value without `=` -- so it cannot swallow
/// an unrelated longer flag.
pub(crate) fn has_glued_short_flag(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|a| {
        flags
            .iter()
            .any(|f| f.len() == 2 && a.starts_with(f) && a.len() > f.len())
    })
}

/// Helper: collect the values following every occurrence of a flag.
///
/// Interpreters like Perl accept multiple `-e`/`-E` fragments and concatenate
/// them at runtime, so analyzing only the first occurrence (`get_flag_value`)
/// misses dangerous code hidden in a later fragment.
pub(crate) fn get_flag_values(args: &[String], flags: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if flags.contains(&args[i].as_str()) {
            if let Some(value) = args.get(i + 1) {
                values.push(value.clone());
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    values
}

/// Collect the value of EVERY occurrence of a flag, across all three
/// spellings value-taking options accept: space-separated (`-o f`,
/// `--output f`), `=`-attached (`--output=f`) and short-glued (`-of`,
/// `-o=/f`).
///
/// SECURITY: `get_flag_value` reads only the first space-separated
/// occurrence, so `--output=/etc/x`, a glued `-o/etc/x` or a second `-o`
/// slipped past output-target checks entirely (#F5). Long options never take
/// a glued value without `=`, so short-glue is restricted to two-char flags;
/// one leading `=` is stripped from a glued short value (pflag-style tools
/// accept `-o=/f`).
pub(crate) fn collect_flag_values(
    args: &[String],
    short_flags: &[&str],
    long_flags: &[&str],
) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if short_flags.contains(&arg.as_str()) || long_flags.contains(&arg.as_str()) {
            if let Some(value) = args.get(i + 1) {
                values.push(value.clone());
            }
            i += 2;
            continue;
        }
        let mut consumed = false;
        for flag in long_flags {
            if let Some(value) = arg
                .strip_prefix(flag)
                .and_then(|rest| rest.strip_prefix('='))
            {
                if !value.is_empty() {
                    values.push(value.to_owned());
                }
                consumed = true;
                break;
            }
        }
        if consumed {
            i += 1;
            continue;
        }
        for flag in short_flags {
            if flag.len() == 2 && arg.starts_with(flag) && arg.len() > flag.len() {
                let raw = &arg[flag.len()..];
                let value = raw.strip_prefix('=').unwrap_or(raw);
                if !value.is_empty() {
                    values.push(value.to_owned());
                }
                break;
            }
        }
        i += 1;
    }
    values
}

/// Detect a flag letter inside a CLUSTERED short-flag token, e.g. perl/ruby's
/// in-place `-i` hidden in `-pi`, `-ipe` or `-pi.bak`.
///
/// SECURITY: perl and ruby accept clustered single-dash flags and let `-i`
/// carry a glued backup suffix (`-pi.bak`), so exact `-i` matching misses
/// in-place edits. Each long-enough single-dash token (not `--long`) is walked
/// as a flag cluster, stopping at:
/// - a value-taking flag letter (`value_flags`) — the rest of the token is
///   that flag's glued value (`-MList::Util`, `-Ilib`), never more flags;
/// - the first non-ASCII-alphabetic byte — for `-i` that is where the backup
///   suffix starts (`-i.bak`).
///
/// Matching is case-sensitive: perl/ruby reserve the uppercase `-I` for
/// include paths, so only the lowercase `flag` letter triggers.
pub(crate) fn has_clustered_short_flag(args: &[String], flag: char, value_flags: &[char]) -> bool {
    args.iter()
        .filter(|a| a.starts_with('-') && !a.starts_with("--"))
        .any(|a| cluster_contains(a, flag, value_flags))
}

/// Walk one token's cluster (chars after the leading `-`) per the rules of
/// [`has_clustered_short_flag`].
fn cluster_contains(arg: &str, flag: char, value_flags: &[char]) -> bool {
    for c in arg.chars().skip(1) {
        if !c.is_ascii_alphabetic() {
            // Backup suffix (`-i.bak`) or other non-flag tail: cluster over.
            return false;
        }
        if value_flags.contains(&c) {
            // Rest of the token is this flag's glued value, not more flags.
            return false;
        }
        if c == flag {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn has_flag_or_prefixed_matches_bare_and_equals_form() {
        let flags = ["--foo"];
        assert!(has_flag_or_prefixed(&args(&["--foo"]), &flags));
        assert!(has_flag_or_prefixed(&args(&["--foo=bar"]), &flags));
        assert!(!has_flag_or_prefixed(&args(&["--foobar"]), &flags));
        assert!(!has_flag_or_prefixed(&args(&["--foobar=x"]), &flags));
        assert!(!has_flag_or_prefixed(&args(&["--other"]), &flags));
    }

    #[test]
    fn has_glued_short_flag_matches_attached_value_only() {
        let flags = ["-I", "-F"];
        assert!(has_glued_short_flag(&args(&["-Ish"]), &flags));
        assert!(has_glued_short_flag(&args(&["-I/bin/sh"]), &flags));
        assert!(has_glued_short_flag(&args(&["-Fscript"]), &flags));
        assert!(!has_glued_short_flag(&args(&["-I"]), &flags));
        assert!(!has_glued_short_flag(&args(&["-i"]), &flags));
        assert!(!has_glued_short_flag(&args(&["--info-script"]), &flags));
    }

    #[test]
    fn sole_help_flag_requires_single_argument() {
        assert!(is_sole_help_flag(&args(&["--help"]), &["--help", "-h"]));
        assert!(!is_sole_help_flag(&args(&["-m", "--help"]), &["--help", "-h"]));
    }

    #[test]
    fn get_flag_values_collects_every_occurrence() {
        let values = get_flag_values(&args(&["-e", "a", "-e", "b"]), &["-e"]);
        assert_eq!(values, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn collect_flag_values_reads_every_spelling_and_occurrence() {
        // Space-separated, `=`-attached, glued short, `=`-glued short, and a
        // second occurrence: get_flag_value sees only the first of these.
        let values = collect_flag_values(
            &args(&["-o", "/tmp/a", "--output=/etc/b", "-o/etc/c", "-o=/d", "-o", "/tmp/e"]),
            &["-o"],
            &["--output"],
        );
        assert_eq!(
            values,
            vec![
                "/tmp/a".to_string(),
                "/etc/b".to_string(),
                "/etc/c".to_string(),
                "/d".to_string(),
                "/tmp/e".to_string(),
            ]
        );
        // Long-flag values via a space, and a value flag that must not
        // swallow a longer unrelated name.
        let values = collect_flag_values(
            &args(&["--input", "/tmp/m", "--input=/tmp/n", "--input-dir=/elsewhere"]),
            &[],
            &["--input"],
        );
        assert_eq!(
            values,
            vec!["/tmp/m".to_string(), "/tmp/n".to_string()]
        );
    }

    #[test]
    fn collect_flag_values_ignores_non_flag_tokens_and_empty_values() {
        assert!(collect_flag_values(&args(&["-o"]), &["-o"], &["--output"]).is_empty());
        assert!(collect_flag_values(&args(&["--output="]), &[], &["--output"]).is_empty());
        // A dash token that merely begins with the flag letter (`-ovation`)
        // is a glued value; a token it does not prefix is skipped.
        let values = collect_flag_values(&args(&["x", "-ovation"]), &["-o"], &["--output"]);
        assert_eq!(values, vec!["vation".to_string()]);
        // Long flag matching stops before `=`: `--output-dir` is not `--output`.
        assert!(collect_flag_values(&args(&["--output-dir=/x"]), &[], &["--output"]).is_empty());
    }

    #[test]
    fn clustered_flag_found_in_clusters_with_and_without_suffix() {
        let value_flags = ['e', 'E', 'I', 'M'];
        assert!(has_clustered_short_flag(&args(&["-pi"]), 'i', &value_flags));
        assert!(has_clustered_short_flag(&args(&["-pi.bak", "-e", "x"]), 'i', &value_flags));
        assert!(has_clustered_short_flag(&args(&["-ipe", "s/a/b/"]), 'i', &value_flags));
        assert!(has_clustered_short_flag(&args(&["-i"]), 'i', &value_flags));
        assert!(has_clustered_short_flag(&args(&["-w", "-i"]), 'i', &value_flags));
    }

    #[test]
    fn clustered_flag_ignores_values_and_other_letters() {
        let value_flags = ['e', 'E', 'I', 'M'];
        // `-Ilib` / `-MList::Util`: uppercase value flags stop the scan, so
        // their glued values are never read as flags.
        assert!(!has_clustered_short_flag(&args(&["-Ilib"]), 'i', &value_flags));
        assert!(!has_clustered_short_flag(&args(&["-MList::Util", "-e", "1"]), 'i', &value_flags));
        // A value flag ends the cluster: letters after it are its value.
        assert!(!has_clustered_short_flag(&args(&["-pei"]), 'i', &value_flags));
        // Plain clusters without the wanted letter.
        assert!(!has_clustered_short_flag(&args(&["-pe", "puts 1"]), 'i', &value_flags));
        // The wanted letter in a separate VALUE token is not a flag.
        assert!(!has_clustered_short_flag(&args(&["-e", "print \"i\""]), 'i', &value_flags));
        // Long options and the bare `-` stdin marker are not clusters.
        assert!(!has_clustered_short_flag(&args(&["--in-place"]), 'i', &value_flags));
        assert!(!has_clustered_short_flag(&args(&["-"]), 'i', &value_flags));
    }
}

/// Collect every positional operand, skipping flags and the bare `--` token.
///
/// `--` ends flag parsing, so tokens after it are operands *even when they
/// start with `-`* (a file literally named `-rf`). Flag tokens themselves
/// (`-r`, `--force`, `-m644`) are skipped, never treated as operands.
/// The token following a flag in `value_flags` is metadata (a mode, an owner,
/// a size) rather than a path, so it is skipped too.
pub(crate) fn positional_operands(
    args: &[String],
    value_flags: &[&str],
) -> Vec<String> {
    let mut operands = Vec::new();
    let mut end_of_flags = false;
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if end_of_flags {
            operands.push(arg.clone());
            continue;
        }
        if arg == "--" {
            end_of_flags = true;
            continue;
        }
        if arg.starts_with('-') {
            if value_flags.contains(&arg.as_str()) {
                skip_next = true;
            }
            continue;
        }
        operands.push(arg.clone());
    }
    operands
}

#[cfg(test)]
mod operand_tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn operands_skip_flags_and_honor_the_double_dash() {
        assert_eq!(
            positional_operands(&v(&["-rf", "/tmp/d"]), &[]),
            vec!["/tmp/d"]
        );
        assert_eq!(
            positional_operands(&v(&["-m644", "/tmp/a", "/tmp/b"]), &[]),
            vec!["/tmp/a", "/tmp/b"]
        );
        // After `--`, dash-leading tokens are operands too.
        assert_eq!(positional_operands(&v(&["--", "-rf"]), &[]), vec!["-rf"]);
        assert_eq!(positional_operands(&v(&["--"]), &[]), Vec::<String>::new());
        assert!(positional_operands(&v(&["-r", "--force"]), &[]).is_empty());
    }

    #[test]
    fn metadata_flag_values_are_skipped_not_treated_as_paths() {
        assert_eq!(
            positional_operands(&v(&["-m", "644", "/tmp/a", "/tmp/b"]), &["-m"]),
            vec!["/tmp/a", "/tmp/b"]
        );
        // An unknown value flag keeps its value visible: conservative.
        assert_eq!(
            positional_operands(&v(&["-x", "644", "/tmp/a"]), &["-m"]),
            vec!["644", "/tmp/a"]
        );
    }
}
