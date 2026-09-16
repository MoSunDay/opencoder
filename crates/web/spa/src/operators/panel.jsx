// panel.jsx — Operator 页签（admin-only，入口在 agentsConfig.jsx）：只读节点
// 总览。会话交互工作台（chat.jsx）创建的会话即 operator 执行——在节点宿主机
// 进程内直接运行 agent loop，非 runc 容器、非节点维护模式；本页展示支持
// operator 的在线节点与负载，实时 transcript 与继续会话在会话交互页完成。

import { Typography } from 'antd';
import { NodeTable } from './nodeTable.jsx';

const { Paragraph } = Typography;

export function OperatorPanel() {
  return (
    <div>
      <Paragraph type="secondary">
        会话交互工作台的会话即以 Operator 运行：在节点宿主机进程内直接执行 agent loop
        （非 runc 容器、非节点维护模式）。本页查看支持 operator 的在线节点与负载；
        执行详情与继续会话在会话交互页完成。
      </Paragraph>
      <NodeTable />
    </div>
  );
}
