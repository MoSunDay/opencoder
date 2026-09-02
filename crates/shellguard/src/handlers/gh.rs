//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{Classification, Handler, HandlerContext, collect_flag_values, is_sole_help_flag};
use crate::verdict::AllowReason;

pub(crate) static GH_HANDLER: GhHandler = GhHandler;

pub(crate) struct GhHandler;

const SAFE_ACTIONS: &[&str] = &[
    "view", "list", "status", "diff", "checks", "search", "download", "watch", "verify", "logs",
    "ports",
];

const UNSAFE_METHODS: &[&str] = &["POST", "PUT", "DELETE", "PATCH"];

/// `gh` subcommands that are read-only whatever follows them.
const TOP_LEVEL_SAFE: &[&str] = &["status", "browse", "search", "completion", "help"];

/// `gh` resource groups whose second token (the action) decides safety.
const RESOURCE_COMMANDS: &[&str] = &[
    "pr",
    "issue",
    "release",
    "repo",
    "run",
    "workflow",
    "gist",
    "project",
    "label",
    "codespace",
    "secret",
    "variable",
];

impl Handler for GhHandler {
    fn commands(&self) -> &[&str] {
        &["gh"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if is_sole_help_flag(ctx.args, &["--help", "-h", "--version"]) {
            return Classification::Allow(AllowReason::handler("gh help/version"));
        }

        let sub = ctx.subcommand();

        match sub {
            "api" => classify_api(ctx),
            sub if TOP_LEVEL_SAFE.contains(&sub) => {
                Classification::Allow(AllowReason::handler(format!("gh {sub}")))
            }
            sub if RESOURCE_COMMANDS.contains(&sub) => classify_resource(ctx, sub),
            _ => Classification::Ask(format!("gh {sub}")),
        }
    }

}

const FIELD_FLAGS: &[&str] = &["-f", "-F", "--raw-field", "--field"];

/// Extract a field flag's value when glued to the flag token: `-ftitle=hi`,
/// `-Ftitle=hi` (pflag shorthand, no separator) or `--field=title=hi`,
/// `--raw-field=title=hi` (long form with `=`). Space-separated forms
/// (`-f title=hi`) are handled separately via `FIELD_FLAGS` + next-token.
fn attached_field_value(arg: &str) -> Option<&str> {
    for long in ["--field=", "--raw-field="] {
        if let Some(v) = arg.strip_prefix(long) {
            return Some(v);
        }
    }
    for short in ["-f", "-F"] {
        if let Some(rest) = arg.strip_prefix(short) {
            let rest = rest.strip_prefix('=').unwrap_or(rest);
            if !rest.is_empty() {
                return Some(rest);
            }
        }
    }
    None
}

fn has_field_flag(args: &[String]) -> bool {
    args.iter()
        .any(|a| FIELD_FLAGS.contains(&a.as_str()) || attached_field_value(a).is_some())
}

fn field_values(args: &[String]) -> impl Iterator<Item = String> + '_ {
    args.iter().enumerate().filter_map(|(i, arg)| {
        attached_field_value(arg).map(str::to_owned).or_else(|| {
            if FIELD_FLAGS.contains(&arg.as_str()) {
                args.get(i + 1).cloned()
            } else {
                None
            }
        })
    })
}

fn classify_api(ctx: &HandlerContext) -> Classification {
    // Every spelling of the method flag (`-X POST`, `--method POST`,
    // `--method=POST`, glued `-XPOST`) and every occurrence — get_flag_value
    // saw only the first space-separated form, so `--method=POST` bypassed
    // the write-method check entirely (#F5).
    for method in collect_flag_values(ctx.args, &["-X"], &["--method"]) {
        if UNSAFE_METHODS.contains(&method.to_uppercase().as_str()) {
            return Classification::Ask(format!("gh api -X {method}"));
        }
    }

    // Check for GraphQL mutation in field arguments
    for val in field_values(ctx.args) {
        if val.contains("mutation") {
            return Classification::Ask("gh api (GraphQL mutation)".into());
        }
    }

    // Non-GraphQL endpoints: field/data flags silently switch the request to
    // POST even though no -X is present, so any field flag means a write.
    let endpoint = ctx.args.get(1).map(String::as_str).unwrap_or_default();
    if endpoint != "graphql" && has_field_flag(ctx.args) {
        return Classification::Ask("gh api (field flag implies write)".into());
    }

    // --input reads from a file — try to inspect contents. All spellings and
    // all occurrences, same as the method flag (#F5).
    for path in collect_flag_values(ctx.args, &[], &["--input"]) {
        if let Some(content) = ctx.read_file(&path) {
            if is_graphql_mutation(&content) {
                return Classification::Ask("gh api --input (GraphQL mutation)".into());
            }
            continue;
        }
        return Classification::Ask("gh api (--input, cannot verify contents)".into());
    }

    Classification::Allow(AllowReason::handler("gh api (GET)"))
}

/// Check if a GraphQL document contains a mutation operation.
fn is_graphql_mutation(content: &str) -> bool {
    // Look for "mutation" as a top-level keyword (not inside a string or comment)
    content
        .split_whitespace()
        .any(|word| word.eq_ignore_ascii_case("mutation") || word.starts_with("mutation{"))
}

fn classify_resource(ctx: &HandlerContext, resource: &str) -> Classification {
    let action = ctx.arg(1);

    if action.is_empty() {
        return Classification::Ask(format!("gh {resource}"));
    }

    if SAFE_ACTIONS.contains(&action) {
        Classification::Allow(AllowReason::handler(format!("gh {resource} {action}")))
    } else {
        Classification::Ask(format!("gh {resource} {action}"))
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::handlers::test_support::{cleanup_dir, temp_dir, write_file};

    // Pure gh command->decision cases (api GET/POST/DELETE/mutation/missing-input,
    // pr/issue actions, status, help) are covered by
    // rippy's command catalog (not ported). Retained below: the GraphQL
    // *query* Allow (the shell parser mangles the `{`-containing arg in the
    // pipeline, so the handler-level Allow is only observable here) and the two
    // --input read_file content tests.

    // gh api tests

    #[test]
    fn api_graphql_query_allows() {
        let args: Vec<String> = vec![
            "api".into(),
            "graphql".into(),
            "-f".into(),
            "query={ repository(owner: \"o\", name: \"r\") { name } }".into(),
        ];
        let result = GH_HANDLER.classify(&HandlerContext::test("gh", &args));
        assert!(matches!(result, Classification::Allow(_)));
    }

    // gh pr tests

    // Top-level commands

    #[test]
    fn api_input_query_file_allows() {
        let dir = temp_dir("fs");
        write_file(
            &dir,
            "query.graphql",
            "{ repository(owner: \"o\", name: \"r\") { name } }",
        );
        let args: Vec<String> = vec![
            "api".into(),
            "graphql".into(),
            "--input".into(),
            "query.graphql".into(),
        ];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("gh", &args)
        };
        let result = GH_HANDLER.classify(&ctx);
        assert!(matches!(result, Classification::Allow(_)));
    }

    #[test]
    fn api_input_mutation_file_asks() {
        let dir = temp_dir("fs");
        write_file(
            &dir,
            "mutate.graphql",
            "mutation { addStar(input: {}) { clientMutationId } }",
        );
        let args: Vec<String> = vec![
            "api".into(),
            "graphql".into(),
            "--input".into(),
            "mutate.graphql".into(),
        ];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("gh", &args)
        };
        let result = GH_HANDLER.classify(&ctx);
        assert!(matches!(result, Classification::Ask(_)));
    }

    /// #F5: `--method=POST` (and every other spelling of `-X`) must hit the
    /// write-method check — get_flag_value saw only the space-separated form.
    #[test]
    fn every_method_spelling_is_checked() {
        for argv in [
            vec!["api", "-X", "POST", "repos/o/r/issues"],
            vec!["api", "--method", "POST", "repos/o/r/issues"],
            vec!["api", "--method=POST", "repos/o/r/issues"],
            vec!["api", "-XPOST", "repos/o/r/issues"],
            vec!["api", "-X", "get", "-X", "DELETE", "repos/o/r/issues"],
        ] {
            let owned: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
            assert!(
                matches!(
                    GH_HANDLER.classify(&HandlerContext::test("gh", &owned)),
                    Classification::Ask(_)
                ),
                "gh {argv:?} must Ask"
            );
        }
        // The read direction stays allowed in the same spellings.
        for argv in [
            vec!["api", "-X", "GET", "repos/o/r"],
            vec!["api", "--method=GET", "repos/o/r"],
            vec!["api", "-XGET", "repos/o/r"],
        ] {
            let owned: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
            assert!(
                matches!(
                    GH_HANDLER.classify(&HandlerContext::test("gh", &owned)),
                    Classification::Allow(_)
                ),
                "gh {argv:?} must Allow"
            );
        }
    }

    /// #F5: `--input=/tmp/m.txt` (the `=` form) must be inspected exactly
    /// like `--input /tmp/m.txt`.
    #[test]
    fn input_file_eq_form_is_inspected() {
        let dir = temp_dir("fs");
        write_file(&dir, "mutate.graphql", "mutation { addStar }");
        for argv in [
            vec!["api", "--input", "mutate.graphql", "graphql"],
            vec!["api", "--input=mutate.graphql", "graphql"],
        ] {
            let owned: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
            let ctx = HandlerContext {
                working_directory: &dir,
                ..HandlerContext::test("gh", &owned)
            };
            assert!(
                matches!(GH_HANDLER.classify(&ctx), Classification::Ask(_)),
                "gh {argv:?} must Ask on a mutation document"
            );
        }
        // Unreadable `=`-form input fails closed too.
        let owned: Vec<String> = vec!["api".into(), "--input=/nope/x.graphql".into(), "repos/o/r".into()];
        assert!(matches!(
            GH_HANDLER.classify(&HandlerContext::test("gh", &owned)),
            Classification::Ask(_)
        ));
        cleanup_dir(&dir);
    }
}
