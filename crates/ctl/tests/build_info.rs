#[test]
fn build_info_matches_platform_without_server_credentials() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_opencoder-cli"))
        .arg("--build-info")
        .env_remove("OPENCODER_SERVER_URL")
        .env_remove("OPENCODER_SERVER_TOKEN")
        .output()
        .unwrap();
    assert!(output.status.success());
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&opencoder_core::version::build_info_json()).unwrap();
    assert_eq!(actual, expected);
    assert!(output.stderr.is_empty());
}
