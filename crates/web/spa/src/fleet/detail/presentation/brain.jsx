import { Button } from 'antd';
import { ReadingTabs } from '../../../ui/runContent/tabs.jsx';
import { useState } from 'react';
import { ExecutionInputs, ExecutionResult } from '../../../ui/runContent/execution.jsx';
import { RawData } from '../../../ui/runContent/value.jsx';
import { DetailFields } from '../fields.jsx';

export function BrainExecutionPresentation({ id, detail, process, events, guidance, context }) {
  const [tab, setTab] = useState('result');
  const [guidanceOpen, setGuidanceOpen] = useState(false);
  return <>
    {guidance && <><Button onClick={() => setGuidanceOpen(!guidanceOpen)} aria-expanded={guidanceOpen}>补充信息</Button>
      {guidanceOpen && <div className="brain-guidance-panel">{guidance}</div>}</>}
    <ReadingTabs activeKey={tab} onChange={setTab} items={[
      { key: 'result', label: '结果', children: <ExecutionResult detail={detail} id={id} /> },
      { key: 'process', label: '执行过程', children: process },
      { key: 'inputs', label: '输入与产物', children: <><ExecutionInputs detail={detail} id={id} />{context}</> },
      { key: 'raw', label: '原始数据', children: <div className="run-content-stack"><RawData value={detail} label="查看执行原始数据" /><DetailFields id={id} detail={detail} />{events}</div> },
    ]} />
  </>;
}
