use super::*;

#[test]
fn interactive_binary_rejected() {
    assert!(is_interactive_command("vim test.txt").is_some());
    assert!(is_interactive_command("nano").is_some());
    assert!(is_interactive_command("top").is_some());
    assert!(is_interactive_command("less file.log").is_some());
}

#[test]
fn bare_repl_rejected() {
    assert!(is_interactive_command("python3").is_some());
    assert!(is_interactive_command("mysql").is_some());
    assert!(is_interactive_command("node").is_some());
}

#[test]
fn noninteractive_allowed() {
    assert!(is_interactive_command("ls -la").is_none());
    assert!(is_interactive_command("python3 -c \"print(1)\"").is_none());
    assert!(is_interactive_command("python3 script.py").is_none());
    assert!(is_interactive_command("mysql -e \"SELECT 1\"").is_none());
    assert!(is_interactive_command("grep foo bar.txt").is_none());
    assert!(is_interactive_command("sudo apt update").is_none());
}

#[test]
fn tmux_name_sanitized() {
    assert_eq!(sanitize_tmux_name("user@1.2.3.4"), "oc-ssh-user-1.2.3.4");
    assert_eq!(sanitize_tmux_name("my-host"), "oc-ssh-my-host");
}

#[test]
fn strip_sudo() {
    assert_eq!(strip_leading_sudo("sudo rm -rf /"), "rm -rf /");
    assert_eq!(strip_leading_sudo("doas vim"), "vim");
    assert_eq!(strip_leading_sudo("sudo doas vim"), "vim");
    assert_eq!(strip_leading_sudo("ls"), "ls");
}

#[test]
fn cmd_base_extracts_binary() {
    assert_eq!(cmd_base("/usr/bin/vim"), "vim");
    assert_eq!(cmd_base("ls -la"), "ls");
    assert_eq!(cmd_base("python3"), "python3");
}

#[test]
fn port_validation_rejects_non_numeric() {
    assert!(validate_port("22").is_ok());
    assert!(validate_port("abc").is_err());
    assert!(validate_port("22; rm -rf /").is_err());
    assert!(validate_port("0").is_err());
    assert!(validate_port("99999").is_err());
    assert!(validate_port("-1").is_err());
}

#[test]
fn shell_injection_validation_rejects_metacharacters() {
    assert!(validate_no_shell_injection("host", "user@1.2.3.4").is_ok());
    assert!(validate_no_shell_injection("key_path", "~/.ssh/id_rsa").is_ok());
    assert!(validate_no_shell_injection("host", "host; rm -rf /").is_err());
    assert!(validate_no_shell_injection("host", "host$(whoami)").is_err());
    assert!(validate_no_shell_injection("port", "22|cat").is_err());
    assert!(validate_no_shell_injection("key_path", "key`id`").is_err());
}

#[test]
fn wrapper_env_stripped_before_denylist() {
    assert!(is_interactive_command("env vim").is_some());
    assert!(is_interactive_command("env FOO=bar vim").is_some());
    assert!(is_interactive_command("exec vim").is_some());
    assert!(is_interactive_command("nohup vim").is_some());
    assert!(is_interactive_command("timeout 10 top").is_some());
    assert!(is_interactive_command("strace vim").is_some());
    assert!(is_interactive_command("strace -f vim").is_some());
}

#[test]
fn wrapper_still_allows_noninteractive() {
    assert!(is_interactive_command("env ls").is_none());
    assert!(is_interactive_command("env FOO=bar grep x file").is_none());
    assert!(is_interactive_command("nohup ls -la").is_none());
    assert!(is_interactive_command("timeout 5 ls").is_none());
}

#[test]
fn nvim_and_other_tuis_rejected() {
    assert!(is_interactive_command("nvim").is_some());
    assert!(is_interactive_command("tmux").is_some());
    assert!(is_interactive_command("gdb ./prog").is_some());
    assert!(is_interactive_command("ranger").is_some());
    assert!(is_interactive_command("lazygit").is_some());
    assert!(is_interactive_command("tig").is_some());
}

#[test]
fn nested_wrappers_unwrapped() {
    assert!(is_interactive_command("env exec vim").is_some());
    assert!(is_interactive_command("sudo env vim").is_some());
    assert!(is_interactive_command("exec env FOO=x vim").is_some());
}

#[tokio::test]
async fn send_without_session_returns_error() {
    use opencoder_core::ToolContext;
    // Clean up first so there's no leftover state for this session id.
    SSH_SESSIONS.lock().unwrap().remove("test-no-session");
    let ctx = ToolContext {
        session_id: "test-no-session".to_string(),
        message_id: "test-msg".to_string(),
        agent: "act".to_string(),
        working_dir: std::path::PathBuf::from("/tmp"),
        max_output: 4096,
        proxy: None,
    };
    let input = serde_json::json!({"action": "send", "command": "ls"});
    let out = SshPtyTool.execute(input, &ctx).await.unwrap();
    // No session registered for this id → error message.
    assert!(out.is_error);
}

#[tokio::test]
async fn connect_rejects_injection_in_host() {
    use opencoder_core::ToolContext;
    // Ensure clean state
    SSH_SESSIONS.lock().unwrap().clear();
    let ctx = ToolContext {
        session_id: "test-injection".to_string(),
        message_id: "test-msg".to_string(),
        agent: "act".to_string(),
        working_dir: std::path::PathBuf::from("/tmp"),
        max_output: 4096,
        proxy: None,
    };
    let input = serde_json::json!({
        "action": "connect",
        "host": "host; rm -rf /tmp"
    });
    let out = SshPtyTool.execute(input, &ctx).await.unwrap();
    assert!(out.is_error);
    // Clean up
    SSH_SESSIONS.lock().unwrap().clear();
}

#[tokio::test]
async fn connect_rejects_injection_in_port() {
    use opencoder_core::ToolContext;
    SSH_SESSIONS.lock().unwrap().clear();
    let ctx = ToolContext {
        session_id: "test-port-injection".to_string(),
        message_id: "test-msg".to_string(),
        agent: "act".to_string(),
        working_dir: std::path::PathBuf::from("/tmp"),
        max_output: 4096,
        proxy: None,
    };
    let input = serde_json::json!({
        "action": "connect",
        "host": "user@1.2.3.4",
        "port": "22; echo pwned"
    });
    let out = SshPtyTool.execute(input, &ctx).await.unwrap();
    assert!(out.is_error);
    SSH_SESSIONS.lock().unwrap().clear();
}

#[tokio::test]
async fn send_rejects_interactive_command() {
    use opencoder_core::ToolContext;
    SSH_SESSIONS.lock().unwrap().clear();
    let ctx = ToolContext {
        session_id: "test-interactive".to_string(),
        message_id: "test-msg".to_string(),
        agent: "act".to_string(),
        working_dir: std::path::PathBuf::from("/tmp"),
        max_output: 4096,
        proxy: None,
    };
    let input = serde_json::json!({
        "action": "send",
        "command": "vim test.txt"
    });
    let out = SshPtyTool.execute(input, &ctx).await.unwrap();
    assert!(out.is_error);
}

#[tokio::test]
async fn status_without_session_reports_none() {
    use opencoder_core::ToolContext;
    SSH_SESSIONS.lock().unwrap().clear();
    let ctx = ToolContext {
        session_id: "test-status-none".to_string(),
        message_id: "test-msg".to_string(),
        agent: "act".to_string(),
        working_dir: std::path::PathBuf::from("/tmp"),
        max_output: 4096,
        proxy: None,
    };
    let input = serde_json::json!({"action": "status"});
    let out = SshPtyTool.execute(input, &ctx).await.unwrap();
    assert!(!out.is_error);
    assert!(out.content.contains("No SSH session registered."));
}

#[tokio::test]
async fn unknown_action_returns_error() {
    use opencoder_core::ToolContext;
    let ctx = ToolContext {
        session_id: "test-unknown".to_string(),
        message_id: "test-msg".to_string(),
        agent: "act".to_string(),
        working_dir: std::path::PathBuf::from("/tmp"),
        max_output: 4096,
        proxy: None,
    };
    let input = serde_json::json!({"action": "foobar"});
    let out = SshPtyTool.execute(input, &ctx).await.unwrap();
    assert!(out.is_error);
    assert!(out.content.contains("Unknown action"));
}
