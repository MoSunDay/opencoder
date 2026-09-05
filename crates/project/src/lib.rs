//! 用户策划的项目跟踪模块：目标(goal) → 里程碑(milestone) → 待办(todo)。
//!
//! 每个待办携带一份粗略草稿(draft)；计划运行(`start_plan`)让 plan 代理把草稿
//! 整理成可执行的实施方案(markdown)，执行运行(`start_execute`)驱动主代理在
//! 工作目录中落地该方案。执行使用「新建或复用」会话策略：同一待办的后续执
//! 行 resume 同一个 session(持续推进)，跨次执行保留完整上下文。每次运行都
//! 以 `project_todo_runs` 行留痕(version 递增)，可取消、可回看。
//!
//! 本 crate 复用 `crates/todos/src/execution.rs` 的「直驱 session」模式
//! (SessionState + run + spawn_event_flusher)，但不复用其 workflow 编排：
//! 这里没有父工作流、候选 JSON 门禁或重试策略——只有 plan/execute 两种
//! 直接驱动的运行。

pub mod context;
mod execute;
pub mod plan_gen;
mod recover;
pub mod service;

pub use context::ProjectContext;
pub use plan_gen::client_for;
pub use service::{Deps, ProjectService};
