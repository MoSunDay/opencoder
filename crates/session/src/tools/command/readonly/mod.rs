//! Compile inspected literal invocations into a controlled read-only script.
use super::powershell::Invocation;
use anyhow::{ensure, Result};
mod git;

fn cmdlet(name: &str) -> Option<String> {
    let module = match name {
        "get-content" | "get-childitem" | "get-item" | "get-itemproperty" | "get-location"
        | "test-path" | "resolve-path" => "Management",
        "select-string" | "select-object" | "measure-object" | "write-output" | "get-filehash" => {
            "Utility"
        }
        _ => return None,
    };
    Some(format!("Microsoft.PowerShell.{module}\\{name}"))
}

pub(super) fn validate(invocation: &Invocation) -> Result<()> {
    let name = invocation.name.to_ascii_lowercase();
    if cmdlet(&name).is_some() {
        return Ok(());
    }
    if name == "git" {
        return git::validate(&invocation.args);
    }
    if name == "rg" {
        ensure!(
            !invocation
                .args
                .iter()
                .take_while(|a| a.as_str() != "--")
                .any(|arg| {
                    (arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains('z'))
                        || [
                            "--pre",
                            "--hostname-bin",
                            "--hyperlink-format",
                            "--search-zip",
                            "-z",
                        ]
                        .iter()
                        .any(|flag| arg == flag || arg.starts_with(&format!("{flag}=")))
                }),
            "rg option may execute an external program"
        );
        return Ok(());
    }
    anyhow::bail!("command is not confirmed read-only: {}", invocation.name)
}

fn byte_offset(source: &str, offset: usize) -> Result<usize> {
    let mut units = 0;
    for (byte, character) in source.char_indices() {
        if units == offset {
            return Ok(byte);
        }
        units += character.len_utf16();
    }
    ensure!(units == offset, "invalid PowerShell source offset");
    Ok(source.len())
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn prepare(source: &str, commands: &[Invocation]) -> Result<String> {
    ensure!(!source.trim().is_empty(), "empty command");
    let mut replacements = Vec::new();
    for invocation in commands {
        validate(invocation)?;
        let name = invocation.name.to_ascii_lowercase();
        let (start, end, replacement) = if let Some(cmdlet) = cmdlet(&name) {
            (invocation.name_start, invocation.name_end, cmdlet)
        } else {
            let arguments = invocation
                .args
                .iter()
                .map(|arg| quote(arg))
                .collect::<Vec<_>>()
                .join(",");
            (
                invocation.start,
                invocation.end,
                format!(
                    "Invoke-OpenCoder{} -Arguments @({arguments})",
                    if name == "git" { "Git" } else { "Rg" }
                ),
            )
        };
        let start = byte_offset(source, start)?;
        let end = byte_offset(source, end)?;
        ensure!(start < end, "invalid PowerShell command span");
        replacements.push((start, end, replacement));
    }
    replacements.sort_unstable_by_key(|(start, _, _)| *start);
    let mut result = source.to_owned();
    let mut boundary = source.len();
    for (start, end, replacement) in replacements.into_iter().rev() {
        ensure!(end <= boundary, "overlapping PowerShell command spans");
        result.replace_range(start..end, &replacement);
        boundary = start;
    }
    Ok(format!("{}\n{result}", include_str!("runtime.ps1")))
}

#[cfg(test)]
mod tests;
