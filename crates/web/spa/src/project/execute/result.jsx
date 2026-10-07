import { Alert, Button, Spin, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../api.js';
import { Markdown } from '../markdown.jsx';

export function ExecutionResult({ id }) {
  const [result, setResult] = useState(null);
  const [error, setError] = useState('');
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let active = true;
    let timer;
    const request = new AbortController();
    setResult(null); setError('');
    const load = async () => {
      try {
        const body = await apiGet(`/api/executions/${encodeURIComponent(id)}/result`, { signal: request.signal });
        if (!active) return;
        setResult(body); setError('');
      } catch (failure) {
        if (!active) return;
        setResult(null);
        setError(failure.status === 503 ? '执行节点离线，恢复后可读取最新结论' : failure.message);
      } finally { if (active) timer = setTimeout(load, 5000); }
    };
    load();
    return () => { active = false; request.abort(); clearTimeout(timer); };
  }, [id, revision]);
  if (error) return <Alert type="error" title={error} action={<Button onClick={() => setRevision((value) => value + 1)}>重试读取</Button>} />;
  if (!result) return <Spin />;
  return <div>
    <Typography.Text strong>执行结论</Typography.Text>
    {result.summary ? <Markdown text={result.summary} /> : <Typography.Paragraph type="secondary">
      {result.kind === 'dag' ? '打开执行明细查看各步骤结果和产物' : '当前执行尚未提供结论'}
    </Typography.Paragraph>}
    {(result.truncated || result.omitted) && <Typography.Paragraph>结论较长，请打开执行明细读取完整结果</Typography.Paragraph>}
    {result.error && <Alert type="error" title={typeof result.error === 'string' ? result.error : JSON.stringify(result.error)} />}
  </div>;
}
