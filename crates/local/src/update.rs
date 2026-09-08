use anyhow::Result;

use crate::run;
use crate::Cli;

/// Built-in prompt that delegates the entire self-update workflow to the agent:
/// clone the latest main, rebuild, and atomically swap the PATH binary.
/// The prompt is in Chinese as specified; the agent executes it via its bash
/// tool (clone -> build -> atomic replace, handling ETXTBSY). It guarantees the
/// agent will NOT kill the running opencoder process itself (no kill/pkill/
/// killall on opencoder), and when the binary is busy (ETXTBSY) it uses the
/// `mv`-based atomic swap rather than destructive means like `rm` or kill.
const UPDATE_PROMPT: &str = "git clone https://github.com/MoSunDay/opencoder.git \
    拉取最新 main 分支代码，完成编译，替换 PATH 中的 opencoder。\
    注意：本次更新任务必然运行在正在执行的 opencoder 进程内部，\
    绝对不要杀死或终止当前 opencoder 进程本身（禁止对 opencoder 使用 kill / pkill / killall）。\
    替换二进制时若遇到 busy（ETXTBSY / Text file busy），必须用 mv 方式处理：\
    先将新构建的二进制 mv 到临时名，再 mv 覆盖目标路径以原子替换；\
    或先将旧运行中的二进制 mv 移走，再写入新的。\
    不要使用 rm 或杀进程等破坏性手段，务必生效新逻辑！";

/// `opencoder update`: run the built-in update prompt headlessly so the agent
/// performs the clone / build / binary-swap steps itself. Reuses `run_headless`
/// so config loading, LLM-client construction, event rendering, Ctrl-C cancel,
/// and title generation are all inherited with zero duplication.
pub async fn update_run(cli: &Cli) -> Result<()> {
    run::run_headless(cli, UPDATE_PROMPT.to_string()).await
}
