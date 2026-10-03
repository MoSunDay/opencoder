use super::*;

fn invocation(name: &str, args: &[&str]) -> Invocation {
    Invocation {
        name: name.into(),
        args: args.iter().map(|s| (*s).into()).collect(),
        start: 0,
        end: 0,
        name_start: 0,
        name_end: 0,
    }
}

#[test]
fn query_options_do_not_admit_execution_or_output_overrides() {
    for args in [
        vec!["diff"],
        vec!["log", "-n5", "--oneline"],
        vec!["log", "--format=%h %s"],
        vec!["status", "--porcelain=v2"],
        vec!["grep", "-c", "needle"],
        vec!["diff", "--", "--output=file"],
    ] {
        validate(&invocation("git", &args)).unwrap();
    }
    for args in [
        vec!["diff", "--textconv"],
        vec!["log", "--show-signature"],
        vec!["log", "--format=%G?"],
        vec!["log", "--pretty=custom"],
        vec!["diff", "--output=file"],
        vec!["grep", "-O"],
        vec!["status", "--recurse-submodules"],
        vec!["-c", "x=y", "status"],
        vec!["diff", "--ext-diff"],
        vec!["diff", "--out=file"],
    ] {
        assert!(validate(&invocation("git", &args)).is_err(), "{args:?}");
    }
    for args in [
        vec!["--pre=helper", "needle"],
        vec!["-z", "needle"],
        vec!["-zn", "needle"],
        vec!["--hyperlink-format=default", "needle"],
    ] {
        assert!(validate(&invocation("rg", &args)).is_err());
    }
    validate(&invocation("rg", &["--", "--pre=literal"])).unwrap();
}

#[test]
fn prepared_commands_preserve_utf16_spans_pipelines_and_literal_arguments() {
    assert!(prepare("  ", &[]).is_err());
    let source = "Write-Output '😀中文'; git diff 'file''s.txt' | Select-Object -First 1";
    let span = |text: &str| {
        let start = source.find(text).unwrap();
        (
            source[..start].encode_utf16().count(),
            source[..start + text.len()].encode_utf16().count(),
        )
    };
    let mut output = invocation("Write-Output", &["😀中文"]);
    (output.name_start, output.name_end) = span("Write-Output");
    let mut git = invocation("git", &["diff", "file's.txt"]);
    (git.start, git.end) = span("git diff 'file''s.txt'");
    let mut select = invocation("Select-Object", &["-First", "1"]);
    (select.name_start, select.name_end) = span("Select-Object");
    let prepared = prepare(source, &[output, git, select]).unwrap();
    assert!(prepared.ends_with("Microsoft.PowerShell.Utility\\write-output '😀中文'; Invoke-OpenCoderGit -Arguments @('diff','file''s.txt') | Microsoft.PowerShell.Utility\\select-object -First 1"));
    assert!(byte_offset("😀", 1).is_err());
}
