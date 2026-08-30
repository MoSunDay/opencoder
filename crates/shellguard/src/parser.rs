//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use rable::Node;

use crate::error::Error;
use crate::nesting::Shape;

/// Stack for the parse thread. rable caps its own parser recursion at 1 000
/// frames, which the worst construct (`case`) needs ~16 MB for; `nesting` caps
/// the lexer recursion rable does not. 256 MB leaves every one of those bounds
/// a margin of 16x or better, and the mapping is only ever committed as it is
/// touched. see docs/security-invariants.md#parser-stack-bound
const PARSE_STACK: usize = 256 * 1024 * 1024;

/// Wrapper around rable bash parser.
pub struct BashParser;

impl BashParser {
    /// Create a new parser.
    ///
    /// # Errors
    ///
    /// Always succeeds — rable is stateless.
    pub const fn new() -> Result<Self, Error> {
        Ok(Self)
    }

    /// Parse a bash command string into a list of AST nodes.
    ///
    /// # Errors
    ///
    /// Returns `Error::TooComplex` for input whose shape rable could only
    /// answer with a stack overflow (#195), and `Error::Parse` if the
    /// source cannot be parsed.
    pub fn parse(&mut self, source: &str) -> Result<Vec<Node>, Error> {
        let shape = Shape::of(source);
        if let Some(detail) = shape.violation() {
            return Err(Error::TooComplex(detail));
        }
        if shape.is_flat() {
            return rable::parse(source, false).map_err(|e| Error::Parse(format!("{e}")));
        }
        parse_on_a_deep_stack(source).map_err(Error::Parse)
    }
}

/// Parse on a thread sized for the bounds in `nesting`. A stack overflow aborts
/// the process rather than unwinding, so the hook's fail-closed net would never
/// see it and the agent would read the empty stdout as an approval (#195).
fn parse_on_a_deep_stack(source: &str) -> Result<Vec<Node>, String> {
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .stack_size(PARSE_STACK)
            .spawn_scoped(scope, || {
                rable::parse(source, false).map_err(|e| format!("{e}"))
            });
        match spawned {
            Ok(handle) => handle
                .join()
                .unwrap_or_else(|_| Err("the parser panicked".to_owned())),
            Err(e) => Err(format!("could not start a parser thread: {e}")),
        }
    })
}

#[cfg(test)]
mod tests {
    use rable::NodeKind;

    use super::*;

    /// Parse `source`, mapping every failure to a message so the tests can
    /// assert with `assert!` instead of `unwrap` (the crate denies it).
    fn parse_result(source: &str) -> Result<Vec<Node>, String> {
        let mut parser = BashParser::new().map_err(|e| e.to_string())?;
        parser.parse(source).map_err(|e| e.to_string())
    }

    /// Parse `source`, failing the test loudly when it cannot be parsed. The
    /// error message cannot be empty, so the assert only fires on failure --
    /// written as a data check because `assert!(false)` is a clippy error.
    fn parse_ok(source: &str) -> Vec<Node> {
        match parse_result(source) {
            Ok(nodes) => nodes,
            Err(msg) => {
                assert!(msg.is_empty(), "parse failed for {source:?}: {msg}");
                Vec::new()
            }
        }
    }

    #[test]
    fn parse_simple_command() {
        let nodes = parse_ok("echo hello");
        assert!(!nodes.is_empty());
        assert!(matches!(nodes[0].kind, NodeKind::Command { .. }));
    }

    #[test]
    fn parse_pipeline() {
        let nodes = parse_ok("cat file | grep pattern");
        assert!(matches!(nodes[0].kind, NodeKind::Pipeline { .. }));
    }

    #[test]
    fn parse_list() {
        let nodes = parse_ok("cd /tmp && ls");
        assert!(matches!(nodes[0].kind, NodeKind::List { .. }));
    }

    #[test]
    fn parse_redirect() {
        let nodes = parse_ok("echo foo > output.txt");
        assert!(
            matches!(&nodes[0].kind, NodeKind::Command { redirects, .. } if !redirects.is_empty())
        );
    }

    #[test]
    fn parse_command_substitution() {
        let nodes = parse_ok("echo $(whoami)");
        assert!(!nodes.is_empty());
        assert!(crate::ast::has_expansions(&nodes[0]));
    }

    #[test]
    fn parse_if_statement() {
        let nodes = parse_ok("if true; then echo yes; fi");
        assert!(matches!(nodes[0].kind, NodeKind::If { .. }));
    }

    #[test]
    fn parse_for_loop() {
        let nodes = parse_ok("for i in 1 2 3; do echo $i; done");
        assert!(matches!(nodes[0].kind, NodeKind::For { .. }));
    }

    #[test]
    fn parse_subshell() {
        let nodes = parse_ok("(echo hello)");
        assert!(matches!(nodes[0].kind, NodeKind::Subshell { .. }));
    }

    #[test]
    fn parse_error_prefix_not_doubled() {
        let msg = match parse_result("echo $( ( unbalanced") {
            Err(msg) => msg,
            Ok(_) => String::from("expected a parse error"),
        };
        // Display prepends exactly one "parse error: "; the closure must not double it.
        assert_eq!(msg.matches("parse error:").count(), 1, "message: {msg}");
    }
}
