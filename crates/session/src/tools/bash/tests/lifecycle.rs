use super::*;

/// A command that exceeds the foreground timeout is handed off to the
/// background supervisor: the output contains the timeout marker, the pid,
/// and the background output file path. The command keeps running (not
/// killed) and is registered for `/ps` / `/stop`.
#[tokio::test]
#[cfg(unix)]
async fn bash_timeout_triggers_handoff() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    // The indirection deliberately avoids a literal `sleep N` hint, so the
    // command keeps the 1 s cfg(test) default and exercises handoff.
    let input = json!({"command": "s=sleep; \"$s\" 3"});
    let out = tool.execute(input, &ctx()).await.unwrap();
    assert!(
        !out.is_error,
        "timeout is not an error — command still runs in background: {}",
        out.content
    );
    assert!(
        out.content.contains(BASH_TIMEOUT_MARKER),
        "timeout output must contain the marker: {}",
        out.content
    );
    assert!(
        out.content.contains("pid:"),
        "timeout output must contain the pid: {}",
        out.content
    );
    assert!(
        out.content.contains("output:"),
        "timeout output must contain the output path label: {}",
        out.content
    );
    assert!(
        out.content.contains("/tmp/opencoder_bg_"),
        "timeout output must reference the background output file: {}",
        out.content
    );
    // Clean up: kill the backgrounded process so it does not linger.
    kill_all();
}

/// The handoff message reports the *display* timeout, not the real one.
/// Guards against a "fix" that switches the message back to the 130 s
/// constant. (Under cfg(test) both are 1 s, so this primarily pins the code
/// path to the display constant; the 120<130 invariant is enforced at
/// compile time by the `const _: () = assert!(...)` in non-test builds.)
#[tokio::test]
#[cfg(unix)]
async fn bash_timeout_message_uses_display_constant() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    // Avoid a literal `sleep N` hint so the cfg(test) default remains 1 s.
    let input = json!({"command": "s=sleep; \"$s\" 3"});
    let out = tool.execute(input, &ctx()).await.unwrap();
    assert!(
        out.content
            .contains(&format!("after {BASH_TIMEOUT_DISPLAY_SECS}s")),
        "handoff message must quote the display constant: {}",
        out.content
    );
    kill_all();
}

/// While a command is running it is registered in the background registry
/// (so `/ps` lists it / `/stop` can kill it); once it exits the entry is
/// removed. Verified by writing the child pid (`$$` == the setsid leader
/// the tool spawned) to a file and inspecting the registry mid-flight.
#[tokio::test]
#[cfg(unix)]
async fn bash_registers_while_running_unregisters_after() {
    let _g = test_registry_mutex().lock().await;
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let tool = BashTool;
    let mut c = ctx();
    c.working_dir = dir.path().to_path_buf();
    // sleep 0.5 finishes well within the 1 s test timeout
    // (BASH_TIMEOUT_SECS == 1 under cfg(test)) so the command completes
    // in the foreground — no handoff, no timeout marker.
    let input = json!({
        "command": format!("echo $$ > {pf}; sleep 0.5; echo done", pf = pidfile.display())
    });
    // Run the tool concurrently so we can inspect the registry mid-flight.
    let handle = tokio::spawn(async move { tool.execute(input, &c).await.unwrap() });

    // Wait for the command to start and write its pid.
    let mut pid: u32 = 0;
    for _ in 0..60 {
        if let Ok(txt) = std::fs::read_to_string(&pidfile) {
            if let Ok(p) = txt.trim().parse::<u32>() {
                pid = p;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(pid > 0, "pidfile never written: command did not start");

    // While running, the live pid must be registered for `/ps` / `/stop`.
    assert!(
        list().iter().any(|i| i.pid == pid),
        "running bash pid {pid} should be registered"
    );

    let out = handle.await.unwrap();
    assert!(out.content.contains("done"), "{}", out.content);

    // After completion the registry entry is removed.
    assert!(
        !list().iter().any(|i| i.pid == pid),
        "completed bash should have unregistered pid {pid}"
    );
}

#[tokio::test]
#[cfg(target_os = "linux")]
async fn dropping_tool_future_kills_descendants_and_unregisters() {
    let _g = test_registry_mutex().lock().await;
    let dir = tempfile::tempdir().unwrap();
    let leader_file = dir.path().join("leader");
    let child_file = dir.path().join("child");
    let tool = BashTool;
    let mut context = ctx();
    context.working_dir = dir.path().to_path_buf();
    let input = json!({
        "command": format!(
            "echo $$ > {}; sleep 300 & echo $! > {}; wait",
            leader_file.display(),
            child_file.display()
        )
    });
    let execution = tokio::spawn(async move { tool.execute(input, &context).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !leader_file.exists() || !child_file.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bash and descendant started");
    let leader: u32 = std::fs::read_to_string(&leader_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let descendant = std::fs::read_to_string(&child_file).unwrap();
    assert!(list().iter().any(|entry| entry.pid == leader));

    execution.abort();
    let _ = execution.await;
    // A killed descendant may linger as an unreaped zombie in /proc (its
    // parent died first and the container init may not reap orphans), so
    // treat the zombie state as dead: the group kill did happen.
    let zombie = |pid: &str| {
        std::fs::read_to_string(std::path::Path::new("/proc").join(pid).join("stat"))
            .ok()
            .and_then(|stat| stat.rsplit(')').next().map(str::to_owned))
            .is_some_and(|rest| rest.trim_start().split(' ').next() == Some("Z"))
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while {
            let path = std::path::Path::new("/proc").join(descendant.trim());
            path.exists() && !zombie(descendant.trim())
        } {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("dropping bash future kills its descendant");
    assert!(
        !list().iter().any(|entry| entry.pid == leader),
        "dropping the future must unregister the bash leader"
    );
}

#[test]
fn parameters_schema_hides_timeout_from_model() {
    // `timeout` is intentionally not a model-facing property: bash derives
    // a bounded deadline from command text and hands long-running commands
    // to the background. Exposing a separate field would restore two
    // competing inputs; `command`/`workdir` stay exposed.
    let schema = BashTool.parameters();
    let props = schema
        .get("properties")
        .expect("schema has a properties object");
    assert!(
        props.get("command").is_some(),
        "command must remain in schema"
    );
    assert!(
        props.get("workdir").is_some(),
        "workdir must remain in schema"
    );
    assert!(
        props.get("timeout").is_none(),
        "timeout must NOT be exposed in the model-facing schema"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn legacy_timeout_prefix_triggers_handoff_and_is_not_executed() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    let input = json!({"command": "timeout 2; sleep 5"});
    let out = tool.execute(input, &ctx()).await.unwrap();

    assert!(!out.is_error, "handoff is not an error: {}", out.content);
    assert!(out.content.contains(BASH_TIMEOUT_MARKER), "{}", out.content);
    assert!(out.content.contains("after 2s"), "{}", out.content);
    assert!(
        !out.content.contains("missing operand"),
        "the silent prefix must be stripped before bash execution: {}",
        out.content
    );
    kill_all();
}

#[tokio::test]
#[cfg(unix)]
async fn legacy_timeout_prefix_widens_default_test_deadline() {
    let _g = test_registry_mutex().lock().await;
    let tool = BashTool;
    let input = json!({"command": "timeout 5; sleep 2; echo ok"});
    let out = tool.execute(input, &ctx()).await.unwrap();

    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("ok"), "{}", out.content);
    assert!(
        !out.content.contains(BASH_TIMEOUT_MARKER),
        "{}",
        out.content
    );
}

#[tokio::test]
async fn legacy_timeout_prefix_with_empty_rest_errors() {
    let out = BashTool
        .execute(json!({"command": "timeout 7;"}), &ctx())
        .await
        .unwrap();

    assert!(out.is_error);
    assert!(out.content.contains("empty command"), "{}", out.content);
}
