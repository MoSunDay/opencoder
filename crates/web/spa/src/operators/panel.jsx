// panel.jsx — Operator 页签（admin-only，入口在 agentsConfig.jsx）：
// 在选定节点宿主机进程内直接运行 agent loop —— 非 runc 容器、非节点
// 维护模式。本文件只管布局与弹层状态：节点表（nodeTable.jsx）→ 启动
// modal（launchModal.jsx）→ 受理后打开 ExecutionDetail 直播 transcript。

import { Typography } from 'antd';
import { useState } from 'react';
import { ExecutionDetail } from '../fleet/detail.jsx';
import { LaunchModal } from './launchModal.jsx';
import { NodeTable } from './nodeTable.jsx';

const { Paragraph } = Typography;

export function OperatorPanel({ onNotice }) {
  const [launch, setLaunch] = useState(null); // 待启动的节点（modal 打开中）
  const [execution, setExecution] = useState(null); // 已受理执行 → 详情抽屉

  return (
    <div>
      <Paragraph type="secondary">
        Operator 在选定节点宿主机进程内直接运行 agent loop（非 runc 容器、非节点维护模式）。
        选择一个支持 operator 的在线节点，指定 agent 与任务要求后启动；执行详情支持实时
        transcript 与继续会话。
      </Paragraph>
      <NodeTable onLaunch={setLaunch} />
      {launch && (
        <LaunchModal
          node={launch}
          onClose={() => setLaunch(null)}
          onAccepted={setExecution}
          onNotice={onNotice}
        />
      )}
      {execution && (
        <ExecutionDetail
          id={execution.id}
          summary={execution}
          onClose={() => setExecution(null)}
          onNotice={onNotice}
        />
      )}
    </div>
  );
}
