//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{Classification, Handler, HandlerContext, get_flag_value, has_flag, has_flag_or_prefixed, is_sole_help_flag};
use crate::verdict::AllowReason;

pub(crate) static CURL_HANDLER: CurlHandler = CurlHandler;

pub(crate) struct CurlHandler;

const DATA_FLAGS: &[&str] = &[
    "-d",
    "--data",
    "--data-raw",
    "--data-binary",
    "--data-urlencode",
    "--data-ascii",
    "-F",
    "--form",
    "-T",
    "--upload-file",
    "--json",
];

const UNSAFE_METHODS: &[&str] = &["POST", "PUT", "DELETE", "PATCH"];

/// curl short-flag characters that combine with `-O`/`-J` in the common
/// "download and save" idiom (`curl -fsSLO url`).
///
/// curl clusters single-dash boolean short options into one token, so
/// `-O`/`-J` can appear glued inside a cluster (`-fsSLO`, `-sJO`) rather than
/// as their own token, bypassing an exact-match check. This list is
/// intentionally narrow (booleans only) so a cluster containing a
/// value-taking short flag (`-X`, `-A`, ...) is never misread as a bundle.
const CURL_BOOLEAN_CLUSTER_FLAGS: &[char] = &[
    'f', 's', 'S', 'L', 'k', 'v', 'i', 'g', 'q', 'n', 'N', '#', '0', '1', '2', '3', '4', '6', 'O',
    'J',
];

/// Detect `-O`/`-J` glued inside a boolean short-option cluster (`-fsSLO`).
fn has_bundled_write_flag(args: &[String]) -> bool {
    args.iter().any(|a| {
        a.starts_with('-')
            && !a.starts_with("--")
            && a.len() > 1
            && a.chars()
                .skip(1)
                .all(|c| CURL_BOOLEAN_CLUSTER_FLAGS.contains(&c))
            && a.chars().skip(1).any(|c| c == 'O' || c == 'J')
    })
}

/// Collect EVERY `-o`/`--output` write target, in the spellings curl accepts:
/// space-separated (`-o f`, `--output f`), `=`-attached (`--output=f`),
/// short-glued (`-of`) and trailing a boolean cluster (`-sSof`, `-sSo f`).
///
/// curl parses a single-dash cluster until a value-taking option letter, so
/// everything after an `o` is the output filename — but only when the letters
/// before it are all known booleans; otherwise the `o` belongs to some other
/// option's glued value and is left alone.
fn curl_output_targets(args: &[String]) -> Vec<String> {
    let mut targets = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--output" {
            if let Some(value) = args.get(i + 1) {
                targets.push(value.clone());
            }
            i += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--output=") {
            if !value.is_empty() {
                targets.push(value.to_owned());
            }
            i += 1;
            continue;
        }
        if arg.starts_with('-') && !arg.starts_with("--") {
            let cluster: Vec<char> = arg.chars().skip(1).collect();
            if let Some(p) = cluster.iter().position(|c| *c == 'o') {
                if cluster[..p].iter().all(|c| CURL_BOOLEAN_CLUSTER_FLAGS.contains(c)) {
                    let glued: String = cluster[p + 1..].iter().collect();
                    if glued.is_empty() {
                        // `-o f` / `-sSo f`: the value rides the next token.
                        if let Some(value) = args.get(i + 1) {
                            targets.push(value.clone());
                        }
                        i += 2;
                        continue;
                    }
                    targets.push(glued);
                    i += 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    targets
}

impl Handler for CurlHandler {
    fn commands(&self) -> &[&str] {
        &["curl"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if is_sole_help_flag(ctx.args, &["--help", "-h", "--version", "-V"]) {
            return Classification::Allow(AllowReason::handler("curl help/version"));
        }

        // Data flags mean a write request
        if has_flag(ctx.args, DATA_FLAGS) {
            return Classification::Ask("curl with data (write request)".into());
        }

        // Explicit unsafe method
        if let Some(method) = get_flag_value(ctx.args, &["-X", "--request"]) {
            if UNSAFE_METHODS.contains(&method.to_uppercase().as_str()) {
                return Classification::Ask(format!("curl -X {method}"));
            }
        }

        // -K/--config
        if has_flag(ctx.args, &["-K", "--config"]) {
            return Classification::Ask("curl --config".into());
        }

        // -o/--output: EVERY write target routes through the redirect
        // pipeline. get_flag_value saw only the first space-separated
        // `-o f`, so `--output=/etc/x`, a glued `-o/etc/x` or a second `-o`
        // slipped past entirely (#F5).
        let outputs = curl_output_targets(ctx.args);
        if !outputs.is_empty() {
            return Classification::WithRedirects(
                AllowReason::handler("curl with output file"),
                outputs,
            );
        }

        // Server-named-write flags: the filename is server-controlled, so we
        // can't emit a redirect target — fail closed rather than Allow.
        if has_flag(
            ctx.args,
            &[
                "-O",
                "--remote-name",
                "--remote-name-all",
                "-J",
                "--remote-header-name",
            ],
        ) || has_flag_or_prefixed(ctx.args, &["--output-dir"])
            || has_bundled_write_flag(ctx.args)
        {
            return Classification::Ask("curl with server-named output (write request)".into());
        }

        Classification::Allow(AllowReason::handler("curl (GET request)"))
    }

}

#[cfg(test)]
mod tests {

    use super::*;

    // curl GET/-d/-X POST/--help command->decision cases are covered by
    // rippy's command catalog (not ported). This test asserts the
    // WithRedirects variant for `-o`, which a command string cannot express.
    #[test]
    fn curl_output_file() {
        let args: Vec<String> = vec![
            "-o".into(),
            "output.html".into(),
            "https://example.com".into(),
        ];
        let result = CURL_HANDLER.classify(&HandlerContext::test("curl", &args));
        assert!(matches!(result, Classification::WithRedirects(..)));
    }

    /// #F5: every output-flag spelling and occurrence must yield a redirect
    /// target — `=`-attached, glued, clustered, and multiple `-o`s.
    #[test]
    fn every_output_spelling_yields_a_target() {
        let targets_of = |args: &[&str]| -> Option<Vec<String>> {
            let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            match CURL_HANDLER.classify(&HandlerContext::test("curl", &owned)) {
                Classification::WithRedirects(_, refs) => Some(refs),
                _ => None,
            }
        };
        assert_eq!(
            targets_of(&["--output=/etc/x", "https://e.com"]),
            Some(vec!["/etc/x".to_owned()])
        );
        assert_eq!(
            targets_of(&["-o/etc/x", "https://e.com"]),
            Some(vec!["/etc/x".to_owned()])
        );
        // Only the first `-o` used to be seen.
        assert_eq!(
            targets_of(&["-o", "/tmp/a", "-o", "/etc/b", "https://e.com"]),
            Some(vec!["/tmp/a".to_owned(), "/etc/b".to_owned()])
        );
        // Boolean cluster with the value glued after `o`.
        assert_eq!(targets_of(&["-sSof", "https://e.com"]), Some(vec!["f".to_owned()]));
        // `o` at the end of a cluster takes the next token.
        assert_eq!(
            targets_of(&["-sSo", "/etc/x", "https://e.com"]),
            Some(vec!["/etc/x".to_owned()])
        );
        // Uppercase `-O` is server-named, never a parsed target.
        assert!(matches!(
            CURL_HANDLER.classify(&HandlerContext::test(
                "curl",
                &["-O".to_owned(), "https://e.com".to_owned()]
            )),
            Classification::Ask(_)
        ));
    }

    /// The redirect pipeline decides released vs non-released targets; the
    /// handler's job is only to surface every target.
    #[test]
    fn output_targets_surface_for_the_redirect_pipeline() {
        for target in ["/tmp/f", "/etc/x"] {
            assert!(matches!(
                CURL_HANDLER.classify(&HandlerContext::test(
                    "curl",
                    &[
                        "-o".to_owned(),
                        target.to_owned(),
                        "https://e.com".to_owned()
                    ]
                )),
                Classification::WithRedirects(..)
            ));
        }
    }
}
