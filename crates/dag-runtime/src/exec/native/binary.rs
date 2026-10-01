use super::super::{StepCtx, StepResult};
use super::io;
use crate::step_log::StepOutputLog;
use opencoder_dag::{StepKind, StepOutcome};
use tokio_util::sync::CancellationToken;

pub async fn execute_binary_step_logged(
    ctx: &StepCtx,
    cancel: CancellationToken,
    output: Option<StepOutputLog>,
) -> StepResult {
    let StepKind::Binary { args, .. } = &ctx.step.kind else {
        return io::error_result("non-binary step passed to binary executor".into());
    };
    if let Err(error) = io::write_context_json(ctx) {
        return io::error_result(format!("prepare step context: {error:#}"));
    }
    let mut argv = vec![format!("/workspace/{}/meta/program", ctx.step.name)];
    argv.extend(args.clone());
    if let Some(serde_json::Value::Array(arguments)) = &ctx.instance_input {
        argv.extend(
            arguments
                .iter()
                .map(|value| value.as_str().expect("validated argument").to_string()),
        );
    }
    let process = crate::sandbox::run::StepProcess {
        key: ctx.execution_key(),
        argv,
        env: io::step_env(ctx),
        cwd: format!("/workspace/{}", ctx.relative_dir()),
        timeout_secs: ctx.step.timeout_secs,
    };
    let result =
        crate::sandbox::run::execute(&io::run_root(ctx), process, cancel.clone(), output).await;
    if let Err(error) = io::archive(ctx, &["output.json", "artifacts.json"]) {
        return io::error_result(format!("archive step output: {error:#}"));
    }
    match result {
        Ok((0, text)) => {
            io::finish_from_output_json(&ctx.dir().expect("validated step path"), text)
        }
        Ok((code, text)) => StepResult {
            output_text: text.clone(),
            ..io::error_result(format!(
                "binary exited with {code}: {}",
                io::tail(&text, 2048)
            ))
        },
        Err(error) => {
            let mut result = io::error_result(format!("{error:#}"));
            if let Some(failure) = error.downcast_ref::<crate::sandbox::run::ProcessFailure>() {
                result.output_text = failure.output.clone();
            }
            if cancel.is_cancelled() {
                result.outcome = StepOutcome::Cancelled;
            }
            result
        }
    }
}
