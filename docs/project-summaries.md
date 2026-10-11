# 项目和专项当前概述

Web 的项目表、专项表及详情展示当前概述和概述更新时间。负责人沿用现有人员设置。概述正文可在编辑表单中维护，也可由 Codex / Operator 读取证据后通过 ctl 写入。

## 更新专项

```sh
opencoder-cli project initiatives summary-context <专项ID> > /tmp/initiative-context.json
```

命令读取专项全部 TODO、每个 TODO 的全部执行关联和原生执行证据，自动继续分页，合并会话正文的分段数据，读取被省略的事件正文及详情字段。同一执行被多个 TODO 关联时只读取一次。原始结果仍归执行节点管理。

根据返回内容整理三个部分：关键进展、核心工作、剩余卡点。保留相关 TODO 和执行 ID，区分已完成、正在进行、待验收。失败尝试不能覆盖后续成功证据，历史已解决的问题不再列为当前卡点。执行成功不等于 TODO 验收完成。

`issues` 列出节点离线、读取失败或结论被省略等需要核对的情况；非空时 `complete` 为 `false`。这些问题必须写进概述或补齐证据后再总结，不能把证据缺失解释为没有进展。运行中的执行可能继续产生新记录，保存前复核变化。

将概述放入 JSON 文件，例如：

```json
{
  "current_summary": "## 关键进展\n- 接口联调已完成，专项验收进行中。\n\n## 核心工作\n- 修复分页读取并补齐回归用例（TODO ID / 执行 ID）。\n\n## 剩余卡点\n- 设备尚未到位，待负责人提供测试设备。"
}
```

```sh
opencoder-cli project initiatives patch <专项ID> --json @/tmp/initiative-summary.json
```

## 更新项目

先按上述步骤更新所属各专项，再读取全部专项进行总结：

```sh
opencoder-cli project goals summary-context <项目ID> > /tmp/project-context.json
opencoder-cli project goals patch <项目ID> --json @/tmp/project-summary.json
```

项目命令返回全部所属专项概述和 TODO 状态，提示空缺概述及概述之后发生的 TODO 变更。项目概述同样填写 `current_summary`，提炼跨专项的关键进展、主要交付与共同卡点；不能用简单平均专项百分比替代项目进度。

服务器生成 `summary_updated_at`，调用方不填写时间。PATCH 不传 `current_summary` 保留原概述和时间；传空字符串清空并记录时间。编辑名称、负责人、状态或说明不刷新概述时间。概述更新沿用原项目写权限，不修改 TODO 状态，不触发新的执行或定时更新。

## 修改记录和回滚

项目列表、专项列表及项目详情中的专项表格提供“概述历史”按钮，详情的当前概述旁也可点击“修改记录”，查看版本内容、保存时间和回滚来源。每个对象默认保留最近 **10 个版本（含当前版本）**，清空概述也会保存版本。新建时保存初始内容，旧对象升级时保存现有内容作为首个版本，方便第一次修改后恢复。

```sh
opencoder-cli project initiatives summary-versions <专项ID>
opencoder-cli project initiatives summary-restore <专项ID> <版本号>
opencoder-cli project goals summary-versions <项目ID>
opencoder-cli project goals summary-restore <项目ID> <版本号>
```

回滚将所选内容保存为一个新版本，并更新概述时间、记录来源版本号；版本号继续递增。正文更新、版本写入和超额清理在同一事务完成，失败时全部撤回。已清理的版本不能回滚。查看沿用项目读权限，回滚需要写权限；删除对象时删除它的概述历史，删除项目不会删除保留专项的历史。
