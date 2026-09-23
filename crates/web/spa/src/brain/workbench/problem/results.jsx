import { Alert, Button, Collapse, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { downloadArtifact } from '../../../fleet/download.js';

const labels = { impact: '影响面', reproduce: '原始复现', repair: '修改', verify: '候选复测', conclude: '产品结论' };
const outcomes = { mapped: '已定位影响面', incomplete: '证据未齐', reproduced: '已复现', not_reproduced: '未复现', blocked: '受阻', changed: '已修改', not_needed: '无需执行', verified: '复测通过', failed: '复测失败', fixed: '已验证修复', diagnosed: '已诊断', unresolved: '未解决' };

export function ProblemResults({ results = [] }) {
  const [error, setError] = useState('');
  if (!results.length) return null;
  const download = async (ref) => {
    try { await downloadArtifact(ref.execution.id, ref.step, ref.file); setError(''); } catch (e) { setError(e.message); }
  };
  return <>{error && <Alert type="error" title={error} />}<Collapse items={results.map(({ stage, round, result, execution_id }) => ({
    key: execution_id,
    label: <Space>{labels[stage]}<Tag color={result.outcome === 'fixed' || result.outcome === 'verified' ? 'green' : 'default'}>{outcomes[result.outcome] || result.outcome}</Tag>第 {round} 轮</Space>,
    children: <><Typography.Paragraph>{result.summary}</Typography.Paragraph>
      {result.reason && <Typography.Paragraph>{result.reason}</Typography.Paragraph>}
      <Space wrap>{(result.evidence || []).filter((e) => e.artifact).map((e, i) => <Button key={`${e.sha256}-${i}`} onClick={() => download(e.artifact)}>{e.path.split('/').at(-1)} · 下载证据</Button>)}</Space>
      <pre style={{ whiteSpace: 'pre-wrap', maxHeight: 320, overflow: 'auto' }}>{JSON.stringify(result, null, 2)}</pre></>,
  }))} /></>;
}
