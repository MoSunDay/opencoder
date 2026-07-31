use anyhow::Result;

use crate::run;
use crate::Cli;

/// Built-in prompt that delegates the entire self-update workflow to the agent:
/// clone the latest main, rebuild, and atomically swap the PATH binary.
/// The prompt is in Chinese as specified; the agent executes it via its bash
/// tool (clone -> build -> atomic replace, handling ETXTBSY).
const UPDATE_PROMPT: &str = "git clone https://github.com/MoSunDay/opencoder.git \
    拉取最新 main 分支代码，完成编译，替换 PATH 中的 opencoder, \
    注意处理 busy 情况，务必生效新逻辑！";

/// `opencoder update`: run the built-in update prompt headlessly so the agent
/// performs the clone / build / binary-swap steps itself. Reuses `run_headless`
/// so config loading, LLM-client construction, event rendering, Ctrl-C cancel,
/// and title generation are all inherited with zero duplication.
pub async fn update_run(cli: &Cli) -> Result<()> {
    run::run_headless(cli, UPDATE_PROMPT.to_string()).await
}
