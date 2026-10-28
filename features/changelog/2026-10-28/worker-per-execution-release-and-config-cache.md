Commit: (working-tree)

# Worker 执行态按终态回收 + 节点配置快照缓存

两项 worker 常驻进程修复，公共行为与协议契约不变：

1. **执行态泄漏回收**：`lifecycle_gates` / `preparation_gates` / `maintenance`
   三张按执行 id 键控的 map 原先只增不删，随执行累计增长。新增自由函数
   `state::release_execution_state`：两张 gate map 用
   `retain(|_, gate| Arc::strong_count(gate) > 1)` 回收（map 自身 Arc 计 1，
   仍持有克隆的在途门控段存活），maintenance 按 id `remove`（Registration
   Drop 释放进程级扩展槽位）。在 `operations/launch.rs` 终态收尾处
   （`active` 移除点，先 drop 任务自身的 gate 克隆再调用）执行回收；
   worker 内仅此一处生产路径移除 `active`。
2. **节点配置快照缓存**：`configuration()` 原先每个报告周期对
   `Config::load` 做候选路径 stat + JSON 解析 + 合并。新增
   `state/config_cache.rs`：纯函数对配置输入文件集（镜像 core 候选发现：
   project/global config.json、opencoder.json 与 5 个域文件、XDG 候选）取
   `max(mtime)+size+count` 指纹；`Inner` 缓存
   `Mutex<Option<(ConfigKey, Arc<Config>)>>`，指纹命中返回同一 Arc，未变则
   跳过加载。harness task-local scope 在位的调用旁路缓存（core 会折入
   scope 覆盖），加载错误不缓存、逐周期继续上抛。

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| 终态回收未引用 gate、在途克隆存活、maintenance 注册释放 | `state::tests::terminal_release_collects_unused_gates_and_maintenance_registrations` |
| 指纹候选集镜像 core 发现布局 | `state::config_cache::tests::candidate_files_mirror_core_discovery_layout` |
| 指纹随文件创建/改写变化、未变时相等 | `state::config_cache::tests::file_stats_stay_equal_until_a_covered_file_changes` |
| 文件删除后指纹回到缺失基线 | `state::config_cache::tests::file_stats_treat_removed_files_as_missing` |
| 缓存命中返回同一 Arc、scope 旁路、文件改写失效 | `state::tests::configuration_cache_reuses_the_load_until_input_files_change` |

## 验证范围

- `cargo test -p opencoder-worker --lib -- state:: stop_signal`：14 用例，13 通过；
  1 例既有环境依赖失败（`native_cancel_reaps_runc_and_descendants_without_model_calls`
  需先运行 `scripts/prepare-dag-rootfs.sh` 准备容器镜像，与本次改动无关）。
- `cargo clippy -p opencoder-worker --all-targets`：零警告。
