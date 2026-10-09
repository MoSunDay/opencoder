import { Alert, Descriptions, Table, Typography } from 'antd';

export function DagRunContext({ context }) {
  if (!context) return null;
  return <div aria-label="DAG 运行环境">
    {context.state === 'preparing' && <Alert type="info" title="资源准备中，尚未固定版本" />}
    {context.state === 'unavailable' && <Alert type="error" title="资源快照缺失，无法确认本次固定版本" />}
    <Descriptions column={1} size="small" items={[
      { key: 'container', label: '容器标识', children: <Typography.Text copyable>{context.container_id}</Typography.Text> },
      { key: 'workspace', label: '共享工作区', children: context.workspace },
    ]} />
    <Table rowKey="name" pagination={false} size="small" dataSource={context.steps} scroll={{ x: 'max-content' }} columns={[
      { title: '步骤', dataIndex: 'name' },
      { title: '容器内工作目录', render: (_, step) => step.dynamic ? `${step.cwd}/instances/<index>` : step.cwd },
      { title: '固定资源', render: (_, step) => step.resource ? `${step.resource.resource || step.resource.name}${step.resource.type === 'binary' ? ` · v${step.resource.version}` : ''}` : context.state === 'preparing' ? '准备中' : '不可用' },
      { title: 'SHA-256', render: (_, step) => step.resource?.sha256 ? <Typography.Text copyable>{step.resource.sha256}</Typography.Text> : '—' },
    ]} />
  </div>;
}
