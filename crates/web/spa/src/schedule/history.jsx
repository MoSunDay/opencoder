import { Alert, Button, Drawer, Table } from 'antd';
import { useState } from 'react';
import { ExecutionDetail } from '../fleet/detail.jsx';
import { useJsonQuery } from '../ui/requests/query.js';
import { StatusTag } from '../ui/statusTag.jsx';
import { TimeText } from '../ui/timeText.jsx';

function readRuns(value) {
  if (!Array.isArray(value?.runs)) throw new Error('触发历史格式错误');
  return value.runs;
}

export function ScheduleRunsDrawer({ schedule, onClose, onNotice }) {
  const { data, loading, error, reload } = useJsonQuery(`/api/schedules/${encodeURIComponent(schedule.id)}/runs?limit=50`, readRuns);
  const [execution, setExecution] = useState(null);
  return <Drawer title={`调度 ${schedule.id} 的触发历史`} size={760} styles={{ wrapper: { maxWidth: '100vw' } }} open onClose={onClose}>
    {error && <Alert type="error" showIcon title={error} action={<Button onClick={reload}>重试触发历史</Button>} />}
    <Table size="small" scroll={{ x: 'max-content' }} rowKey="scheduled_for_ms" loading={loading} dataSource={data || []}
      locale={{ emptyText: error ? '触发历史读取失败，请重试' : '暂无触发记录' }} columns={[
        { title: '计划时间', dataIndex: 'scheduled_for_ms', render: (value) => <TimeText ts={value} /> },
        { title: '实际触发', dataIndex: 'fired_at_ms', render: (value) => <TimeText ts={value} /> },
        { title: '状态', dataIndex: 'status', render: (value) => <StatusTag status={value} /> },
        { title: '执行 ID', dataIndex: 'execution_id', render: (value) => value ? <Button type="link" onClick={() => setExecution(value)}>{value}</Button> : '—' },
        { title: '失败原因', dataIndex: 'error', render: (value) => value || '—' },
      ]} />
    {execution && <ExecutionDetail id={execution} onClose={() => setExecution(null)} onNotice={onNotice} />}
  </Drawer>;
}
