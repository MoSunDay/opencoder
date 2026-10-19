import { Alert, Button, Space, Table, Typography } from 'antd';
import { useState } from 'react';
import { Markdown } from '../../project/markdown.jsx';
import { PayloadWindows } from '../../fleet/detail/fields.jsx';
import { downloadArtifact } from '../../fleet/download.js';
import './style.css';

export function ReadableText({ text, markdown = false }) {
  const [expanded, setExpanded] = useState(false);
  const value = String(text ?? '');
  const long = value.length > 1200;
  const shown = long && !expanded ? `${value.slice(0, 1200)}…` : value;
  return <div className="run-readable"><div className="run-readable-body">{markdown ? <Markdown text={shown} /> : <span className="run-plain-text">{shown}</span>}</div>
    {(long || value.length > 100) && <Space>{long && <Button type="link" size="small" onClick={() => setExpanded(!expanded)}>{expanded ? '收起内容' : '展开全文'}</Button>}
      <Typography.Text copyable={{ text: value }}>复制内容</Typography.Text></Space>}
  </div>;
}

export function RawData({ value, label = '查看原始数据' }) {
  const [open, setOpen] = useState(false);
  const text = typeof value === 'string' ? value : JSON.stringify(value, null, 2);
  return <div className="run-raw"><Button size="small" onClick={() => setOpen(!open)} aria-expanded={open}>{open ? '收起原始数据' : label}</Button>
    {open && <><Typography.Text copyable={{ text: text ?? '' }}>复制当前数据</Typography.Text><pre>{text ?? '未记录'}</pre></>}
  </div>;
}

function Artifact({ value }) {
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  const download = async () => {
    setBusy(true); setError('');
    try { await downloadArtifact(value.execution.id, value.step, value.file, value.index); }
    catch (e) { setError(e.message); } finally { setBusy(false); }
  };
  return <div className="run-artifact"><Space wrap><Typography.Text strong>{value.file}</Typography.Text><span>{value.bytes} 字节</span>
    <Button size="small" loading={busy} onClick={download}>下载文件</Button></Space>
    <Typography.Text type="secondary">所属执行：{value.execution.id} · {value.step}{value.index != null ? ` · 实例 ${value.index}` : ''}</Typography.Text>
    <RawData value={value} label="文件引用详情" />{error && <Alert type="error" title={error} />}</div>;
}

export function DataValue({ value, id, depth = 0 }) {
  const [expanded, setExpanded] = useState(depth < 2);
  const [limit, setLimit] = useState(20);
  if (value === undefined) return <Typography.Text type="secondary">未记录</Typography.Text>;
  if (value === null) return <code>null</code>;
  if (typeof value === 'string') return value === '' ? <code>空字符串</code> : <ReadableText text={value} />;
  if (typeof value !== 'object') return <code>{String(value)}</code>;
  if (value.omitted === true && value.read_via === 'detail_field') return <div><Typography.Text type="secondary">内容较大，按段读取</Typography.Text><PayloadWindows key={`${id}:${value.field}`} id={id} marker={value} /></div>;
  if (value.execution?.id && typeof value.step === 'string' && typeof value.file === 'string' && typeof value.bytes === 'number' && typeof value.sha256 === 'string') return <Artifact value={value} />;
  const entries = Object.entries(value);
  if (!entries.length) return <code>{Array.isArray(value) ? '[]' : '{}'}</code>;
  if (!expanded) return <Button size="small" onClick={() => setExpanded(true)}>展开 {entries.length} 项</Button>;
  const rows = Array.isArray(value) && value.every((row) => row && !Array.isArray(row) && typeof row === 'object'
    && !row.omitted && !row.execution && Object.keys(row).sort().join('\0') === Object.keys(value[0]).sort().join('\0'));
  return <div className="run-data-value">
    {depth >= 2 && <Button size="small" type="link" onClick={() => setExpanded(false)}>收起 {entries.length} 项</Button>}
    {rows ? <Table size="small" pagination={false} rowKey="_row" scroll={{ x: 'max-content' }}
      dataSource={value.slice(0, limit).map((row, index) => ({ _row: index, data: row }))}
      columns={Object.keys(value[0]).map((key) => ({ title: key, key, render: (_, row) => <DataValue value={row.data[key]} id={id} depth={depth + 1} /> }))} />
      : <dl className="run-data-fields">{entries.slice(0, limit).map(([key, item]) => <div key={key}><dt>{Array.isArray(value) ? Number(key) + 1 : key}</dt><dd><DataValue value={item} id={id} depth={depth + 1} /></dd></div>)}</dl>}
    {entries.length > limit && <Button onClick={() => setLimit(limit + 20)}>继续显示（{limit} / {entries.length}）</Button>}
  </div>;
}
