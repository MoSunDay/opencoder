// panel.jsx — 「节点总览」页签（admin-only，入口在 agentsConfig.jsx）：只读节点
// 总览。Agent 页（nav menu 已改名 Agent，page key 仍为 chat，chat.jsx）创建的
// 会话即 operator 执行——在节点宿主机进程内直接运行 agent loop，非 runc 容器、
// 非节点维护模式；本页展示支持 operator 的在线节点与负载，实时 transcript 与
// 继续会话在 Agent 页完成。

import { Typography } from 'antd';
import { NodeTable } from './nodeTable.jsx';

const { Paragraph } = Typography;

export function OperatorPanel() {
  return (
    <div>
      <Paragraph type="secondary">
        Agent 分类下的 Agent 页即以 Operator（operator 执行类型）运行会话：在节点宿主机进程内直接执行
        agent loop（非 runc 容器、非节点维护模式）。本页查看支持 operator 的在线节点
        与负载；执行详情与继续会话在 Agent 页完成。
      </Paragraph>
      <NodeTable />
    </div>
  );
}
