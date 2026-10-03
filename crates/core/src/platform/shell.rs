//! The command language follows the execution host, never the frontend.
pub const fn tool_name() -> &'static str {
    if cfg!(windows) {
        "powershell"
    } else {
        "bash"
    }
}

pub fn prompt(text: &str) -> String {
    if cfg!(windows) {
        text.replace("bash", "powershell")
            .replace("cat/grep/sed", "Get-Content/Select-String")
            + "\nCommands execute in PowerShell 7 on Windows. Use PowerShell syntax and Windows paths; do not assume Unix utilities are installed."
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_tool_and_prompt_agree() {
        let text = super::prompt("Use bash and cat/grep/sed");
        assert!(text.contains(super::tool_name()));
        if cfg!(windows) {
            assert!(text.contains("PowerShell 7"));
            assert!(!text.contains("bash"));
        } else {
            assert_eq!(text, "Use bash and cat/grep/sed");
        }
    }
}
