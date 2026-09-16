// wasm step log view: node-side stdout/stderr capture rendered in the same
// bounded console look as the run-wide logs (run.css `.execution-log-lines`,
// loaded by dag/run/result.jsx). Search + auto-scroll mirror executionLogs.jsx.
import { Input, Space, Switch, Tag, Typography } from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import { outputRows } from './model.js';

const { Text } = Typography;
const CONNECTION = { connecting: '加载记录中', open: '已连接', live: '实时', reconnecting: '重连中', closed: '已结束', failed: '连接失败' };

export function WasmLogs({ frames = [], trimmed = false, connection }) {
  const [query, setQuery] = useState('');
  const [autoScroll, setAutoScroll] = useState(true);
  const log = useRef(null);
  const rows = useMemo(() => outputRows(frames, query), [frames, query]);
  useEffect(() => {
    if (autoScroll && log.current) log.current.scrollTop = log.current.scrollHeight;
  }, [rows, autoScroll]);
  return <div className="execution-logs">
    <Space wrap>
      <Input aria-label="搜索输出" allowClear placeholder="搜索输出" value={query} onChange={(e) => setQuery(e.target.value)} style={{ width: 220 }} />
      <Switch checked={autoScroll} onChange={setAutoScroll} aria-label="自动滚动" /><Text>自动滚动</Text>
      <Tag>{CONNECTION[connection] || '连接中'}</Tag>
      {trimmed && <Text type="secondary">已截断早期输出</Text>}
    </Space>
    <div ref={log} role="log" className="execution-log-lines">
      {rows.length ? rows.map((row, index) => (
        <div key={`${row.seq}:${index}`}>
          <Text type={row.stream === 'stderr' ? 'warning' : 'secondary'}>[{row.stream}] </Text>
          {row.text}
        </div>
      )) : <Text type="secondary">该步骤还没有执行输出</Text>}
    </div>
  </div>;
}
