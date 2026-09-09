import { Alert, Button, Card, Descriptions, Space, Tag, Typography } from 'antd';
import { downloadArtifact } from '../download.js';
import { err } from '../../notice.js';

const STAGES = { starting: '准备执行', collecting: '采集证据', tracing: '关联日志', engineering_context: '读取工程上下文',
  agent_analysis: 'Codex 归因', resolving_versions: '固定代码版本', snapshotting: '准备快照', diff: '分析变更',
  impact_analysis: 'Codex 初审', verification: '执行回归测试', review: 'Codex 复核', reporting: '生成报告' };
const VERDICTS = { pass: ['green', '通过'], block: ['red', '阻断'], inconclusive: ['orange', '证据不足'] };

export function RunnerDetail({ id, detail, onNotice }) {
  const input = detail?.request?.input || {}, annotations = detail?.annotations || {};
  const download = async (step, file) => {
    try { await downloadArtifact(id, step, 'artifacts/' + file); }
    catch (error) { onNotice(err(error.message)); }
  };
  return (detail?.runners || []).map((entry) => {
    const pinned = entry.configuration || {}, verdict = VERDICTS[entry.verdict];
    const resources = Object.entries(pinned.resources || {}).map(([kind, value]) => `${kind}: ${value?.name || value} ${value?.version || ''}`).join('；');
    return <Card key={entry.step} size="small" title={`${entry.agent} · ${entry.step}`} style={{ marginBottom: 16 }}>
      <Descriptions size="small" column={2} items={[
        { key: 'job', label: '业务任务', children: input.job_id || annotations.job_id || entry.detail?.job_id || '—' },
        { key: 'attempt', label: '尝试', children: input.attempt || annotations.attempt || entry.detail?.attempt || '—' },
        { key: 'stage', label: '执行阶段', children: detail.execution?.status === 'pending' ? '等待节点名额' : STAGES[entry.stage] || entry.stage || '—' },
        { key: 'runner', label: 'Runner', children: `${entry.runner}${pinned.revision ? ` · v${pinned.revision}` : ''}` },
        { key: 'profile', label: 'Codex 配置', children: `${pinned.profile || '默认配置'}${pinned.profile_revision ? ` · v${pinned.profile_revision}` : ''}` },
        { key: 'delivery', label: '报告投递', children: annotations.delivery_status || '—' },
        { key: 'resources', label: '资源快照', span: 2, children: resources || '接收任务时固定' },
      ]} />
      {['error', 'cancelled', 'interrupted'].includes(detail.execution?.status) && <Alert type="info" title="本次尝试已结束" description="通过原业务任务的重试接口创建新尝试；旧执行记录和报告保留。" />}
      {verdict && <Space style={{ marginTop: 10 }}><Typography.Text>回归准出结论</Typography.Text><Tag color={verdict[0]}>{verdict[1]}</Tag></Space>}
      {entry.summary && <Typography.Paragraph style={{ marginTop: 10 }}>{entry.summary}</Typography.Paragraph>}
      {annotations.delivery_error && <Alert type="error" title={`投递失败：${annotations.delivery_error}`} />}
      <Space wrap style={{ marginTop: 10 }}>{(entry.artifacts || []).filter((a) => /(?:report\.(?:json|html|md)|attribution\.(?:json|html|md)|result\.json)$/.test(a.file)).map((a) =>
        <Button key={a.file} size="small" onClick={() => download(entry.step, a.file)}>{a.file}</Button>)}</Space>
    </Card>;
  });
}
