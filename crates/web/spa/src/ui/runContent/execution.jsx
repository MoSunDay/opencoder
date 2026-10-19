import { Alert, Empty, Typography } from 'antd';
import { DataValue, RawData, ReadableText } from './value.jsx';

export function ExecutionResult({ detail, id }) {
  const result = detail?.result;
  const hasOutput = result != null && Object.hasOwn(result, 'scheduler_output');
  const output = hasOutput ? result.scheduler_output : result;
  const running = ['pending', 'running', 'idle', 'cancelling'].includes(detail?.execution?.status);
  return <div className="run-content-stack">
    {detail?.error && <Alert type="error" showIcon title="执行异常" description={<ReadableText text={detail.error} />} />}
    {running && <Alert type="info" showIcon title="执行尚未结束" description="可以切换到执行过程查看当前进展。" />}
    {output == null && !hasOutput ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={running ? '尚未生成最终结果' : '未记录结构化结果，可查看执行过程'} />
      : typeof output === 'string' ? <ReadableText text={output} markdown /> : <DataValue value={output} id={id} />}
  </div>;
}

export function ExecutionInputs({ detail, id }) {
  const input = detail?.request?.input;
  const frozen = input?.layered_request;
  const actual = frozen ? frozen.inputs : input;
  return <div className="run-content-stack">
    <section><Typography.Title level={5}>本次实际输入</Typography.Title><DataValue value={actual} id={id} /></section>
    <section><Typography.Title level={5}>已登记产物</Typography.Title>
      {detail?.result?.omitted ? <DataValue value={detail.result} id={id} />
        : detail?.result?.scheduler_artifacts == null ? <Typography.Text type="secondary">未记录产物引用</Typography.Text> : <DataValue value={detail.result.scheduler_artifacts} id={id} />}</section>
    {frozen?.artifacts && <section><Typography.Title level={5}>输入文件引用</Typography.Title><DataValue value={frozen.artifacts} id={id} /></section>}
    <RawData value={detail?.request} label="查看原始请求" />
  </div>;
}
