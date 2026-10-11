Commit: b490f40bc194ee00196baaaef1362a5202be8a8a

# 专项不再维护自身状态

专项只组织 TODO，不再保存 `planned/in_progress/done` 状态。删除领域类型和存储字段、列表状态列以及所有专项状态编辑入口；专项创建、PATCH 拒绝 `status` 字段，Web、Control 和 ctl 共用同一行为。专项与项目进度仍由 TODO 看板状态计算，项目归档不变。

schema v38 在事务中移除 `project_initiatives.status`，保留归属、排序、概述、人员、Tag、TODO 和执行关联。历史里程碑迁移不再复制专项状态。数据契约升为 9，原有数据契约必须经过维护升级，避免旧 Server 读取已移除字段。发布清单接受数据契约 9；维护预检与项目表恢复支持 schema v38。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 移除旧专项状态，数据与 TODO 进度不变 | `v37_removes_only_initiative_status_and_preserves_todo_progress` | [迁移测试](../../../crates/store/tests/initiative_without_status.rs) |
| 迁移失败回滚 | `failed_status_migration_rolls_back_schema_and_rows` | [迁移测试](../../../crates/store/tests/initiative_without_status.rs) |
| Web 拒绝旧字段，保留其他编辑 | `initiatives_reject_status_and_keep_people_summary_and_todo_board_writes` | [HTTP 测试](../../../crates/web/tests/web_initiative_status.rs) |
| Control 拒绝状态，不能部分修改 | `initiative_status_is_rejected_without_partial_updates` | [Control 测试](../../../crates/control/tests/e2e/project_crud_extra/suite_1.rs) |
| 专项列表、编辑、详情和项目内新建 | `initiativeStatus.dom.test.jsx` 的三个用例 | [界面测试](../../../crates/web/spa/src/project/views/initiativeStatus.dom.test.jsx) |
| 恢复旧项目结构且不修改认证数据 | `test_status_removal_can_restore_previous_project_shape_without_auth_writes` | [恢复测试](../../../scripts/platform/maintenance_tests/test_initiative_status.py) |
| 不兼容版本禁止滚动升级 | `initiative_schema_requires_maintenance_from_previous_formats` | [版本约定](../../../crates/core/src/fleet/release.rs) |
| 真实 Server、ctl、四种屏宽 | `node scripts/acceptance/initiative_status/main.cjs <bin-dir> <新证据目录>` | [验收入口](../../../scripts/acceptance/initiative_status/main.cjs) |

定向测试范围为专项 CRUD、概述、人员、Tag、项目进度、历史数据库迁移及相关维护发布检查；没有运行全量回归。主工作区额外验证专项排序与概述历史，保留已有编辑。

实际测试日志、构建清单、真实浏览器截图、发布回执、数据比对及 15 分钟线上观察保存在 `/data00/workspace/artifacts/opencoder-initiative-without-status-20261010`。

定向回归：Rust 103 项、SPA 22 项通过；主工作区排序与概述历史相关 16 项通过；维护预检及恢复 8 项通过。相关五个 Rust crate 的 `cargo clippy --release --all-targets -- -D warnings` 零警告，SPA 产物无漂移。真实 Server、ctl 和 1920/1280/768/390 宽度验收通过；CLI 拒绝信息从 stderr 读取。

发布清单格式校验定向回归：`python3 -m unittest discover -s scripts/platform/rolling_tests -p test_manifest.py`，包括格式 9 的构建校验与禁止和格式 1–8 滚动混用。

线上结果：已在原入口 `http://10.199.71.70:18081` 将本次修改合入现网版本 `b490f40b`，发布为 `fe01be20`。schema v38 已移除专项状态；项目、专项、TODO、关联和认证表逐行摘要比对通过。原 4 个节点恢复，四种屏宽的列表和表单通过线上复核。连续观察 903 秒、167 次状态与进度检查、15 次真实执行探测全部通过。最终回执：`/data00/workspace/artifacts/opencoder-initiative-without-status-20261010/final-receipt.json`。
