Commit: 2686d40a267436adb555041fc8154fc9bb454574 (working-tree)

# Brain 计划本体编辑与条件执行

- 新建计划使用右侧 100% 画布；编辑中的本体、画布位置和视口写入浏览器草稿，提交成功后才清除。
- 名称与一句话概述在提交对话框填写；缓存写入失败会阻止关闭和提交。
- Action 绑定能力库实体，输入端口明确来源和类型；条件流转支持默认边、现象判断、回退和完成交付物校验。
- 运行时为回退动作创建新的持久化访问轮次，复测失败回到修复，只有通过分支才发布；重复回执、暂停、取消和轮次上限均有边界测试。

验证映射：`crates/brain/tests/action_flow.rs`（领域流转、回执幂等、暂停取消和限制）；`crates/worker/tests/brain_flow.rs`、`brain_ontology.rs`（节点持久化轮次与控制面回归）；`crates/web/spa/src/brain/workbench/tests/plans.dom.test.jsx`、`draft.test.js`（画布草稿、提交对话框、缓存失败和本体图）。
