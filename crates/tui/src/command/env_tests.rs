use super::*;

#[test]
fn env_commands_parse_operations_and_aliases() {
    let cases = [
        ("/env", EnvCommand::Show),
        ("/env list", EnvCommand::Show),
        ("/envs", EnvCommand::Show),
        ("/env alpha", EnvCommand::Activate("alpha".into())),
        ("/env use alpha", EnvCommand::Activate("alpha".into())),
        ("/env activate alpha", EnvCommand::Activate("alpha".into())),
        ("/env create alpha", EnvCommand::Create("alpha".into())),
        ("/env new alpha", EnvCommand::Create("alpha".into())),
        ("/env delete alpha", EnvCommand::Delete("alpha".into())),
        ("/env remove alpha", EnvCommand::Delete("alpha".into())),
        ("/env rm alpha", EnvCommand::Delete("alpha".into())),
        ("/env off", EnvCommand::Deactivate),
        ("/env none", EnvCommand::Deactivate),
        ("/env deactivate", EnvCommand::Deactivate),
    ];
    for (input, expected) in cases {
        assert_eq!(parse(input), Some(SlashAction::Env(expected)), "{input}");
    }
}

#[test]
fn env_commands_reject_missing_or_extra_operands() {
    for input in [
        "/env use",
        "/env create",
        "/env delete one two",
        "/env off now",
    ] {
        match parse(input) {
            Some(SlashAction::Env(EnvCommand::Invalid(message))) => {
                assert!(message.starts_with("usage: /env"), "{input}: {message}");
            }
            other => panic!("{input} should produce an Env usage error, got {other:?}"),
        }
    }
    assert_eq!(parse("/envs extra"), None);
}
