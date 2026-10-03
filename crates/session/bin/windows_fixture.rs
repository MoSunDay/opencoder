//! Native executable fixture for Windows supervisor and Codex tests.
use std::io::{Read, Write};
pub fn dispatch() -> bool {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "internal-process-supervisor")
    {
        assert_eq!(args[1], "--");
        let code = opencoder_session::process::supervisor_main(args[2..].to_vec(), None).unwrap();
        std::process::exit(code);
    }
    if args.first().is_some_and(|arg| arg == "--job-parent") {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let binary = std::env::current_exe().unwrap();
            opencoder_session::process::configure_supervisor_binary(binary.clone()).unwrap();
            let (mut command, lease) = opencoder_session::process::command(binary)
                .unwrap()
                .unwrap();
            command
                .arg("--job-child")
                .arg(&args[1])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let mut child = command.spawn().unwrap();
            let _owner = lease.spawned(child.id()).unwrap();
            child.wait().await.unwrap();
        });
        return true;
    }
    if args.first().is_some_and(|arg| arg == "--job-child") {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--sleep-child")
            .spawn()
            .unwrap();
        std::fs::write(&args[1], child.id().to_string()).unwrap();
        child.wait().unwrap();
        return true;
    }
    if args.first().is_some_and(|arg| arg == "--sleep-child") {
        std::thread::sleep(std::time::Duration::from_secs(120));
        return true;
    }
    if !args.first().is_some_and(|arg| arg == "exec") {
        return false;
    }
    let args: Vec<_> = args
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).unwrap();
    let capture = std::env::var("CAPTURE").unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(capture)
        .unwrap();
    writeln!(
        file,
        "{}",
        serde_json::json!({"args":args,"prompt":prompt,"home":std::env::var("USERPROFILE").ok()})
    )
    .unwrap();
    for event in [
        serde_json::json!({"type":"thread.started","thread_id":"windows-fixture-thread"}),
        serde_json::json!({"type":"turn.started"}),
        serde_json::json!({"type":"item.completed","item":{"id":"a1","type":"agent_message","text":"native answer"}}),
        serde_json::json!({"type":"turn.completed","usage":{"input_tokens":12,"output_tokens":5,"cached_input_tokens":0}}),
    ] {
        println!("{event}");
    }
    true
}
