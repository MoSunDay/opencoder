Commit: 本笔(测试契约修复,随下一批发布)

# 调度器基线失败复核:brain 无 objective 由 config 校验拒收——测试对齐跳过契约

rel-4676663c 发布豁免的两条基线失败之一 `schedule_api::brain_schedule_without_objective_records_an_error_row`(`crates/control/tests/e2e/schedule_api.rs:300`)人工复核完成:**非 cron tick 回归,无需回退**;是 `272b2b13` 同一提交内实现与测试的契约矛盾,已修测试对齐现行契约。

## 根因

- `scan`(`crates/control/src/scheduler.rs`)对每个 job 先跑 `ScheduleJob::validate()`,失败仅 `warn!("invalid schedule skipped")` 并 `continue`——不 fire、不写 `schedule_runs` 台账(fail-soft,坏 job 不得饿死其它)。
- `ScheduleJob::validate`(`crates/core/src/config/schedule.rs`)对 `kind=brain` 调 `validate_brain_params`,要求 `objective` 非空字符串;注释明示意图:「bad objective / mode fails here instead of landing as an `error` ledger row」。
- 旧测试却期望 fire-time dispatch 失败落一条 `status=error` 行(30s 轮询)——该路径在 config 校验存在后**不可达**,轮询必然超时。两提交同日落地即互相矛盾,与发布豁免记录的「065c017b 基线同样稳定失败」一致。
- 附:调度器对 brain 空参数的 fire-time 兜底(`brain_run()` 报 "needs an objective" → error 行 + 1h 重试)仍在,仅对 validate 放行后的场景可达(如 enabled 在 fire 间隔被改坏)。

## 修复(仅测试,无产品代码改动)

`crates/control/tests/e2e/schedule_api.rs`:重写为 `brain_schedule_without_objective_is_skipped_without_starving_siblings`,断言现行契约:

1. 同文件混布非法 brain job(`params:{}`)+ 合法 agent job → 合法兄弟照常 fire(fail-soft 隔离);
2. `/api/schedules` 仍列出非法 job(定义源是文件),`last_run` 为 null、`next_run` 正常(cron 本身合法);
3. `/api/schedules/brain_no_obj/runs` 恒为空数组——无 dispatch 即无 error 行。

另一条豁免 `gating::non_admin_role_gates_the_surface` 属并行会话进行中工作(角色门禁投影改造),不在本笔范围。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| brain 无 objective 拒收 + fail-soft 隔离 | `schedule_api::brain_schedule_without_objective_is_skipped_without_starving_siblings` | `crates/control/tests/e2e/schedule_api.rs` |
| schedule_api 全文件回归 | `cargo test -p opencoder-control --test e2e schedule_api` → 7 passed / 0 failed | 同上 |

## 相关

- [release-4676663c-signal-deploy](release-4676663c-signal-deploy.md)(豁免出处)、[control-cron-scheduler-agent-sessions](control-cron-scheduler-agent-sessions.md)(调度器落地)。
