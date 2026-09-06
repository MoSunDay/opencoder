import { Alert, Button, Space, Spin, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../api.js';
import { decodeBase64, decodeWindow } from '../model.js';

const LABELS = {
  'request.input': '请求内容',
  definition: '执行定义',
  result: '执行结果',
  'workflow.spec_json': '工作流定义',
  'workflow.state_json': '工作流状态',
  'team.topic': '团队协作内容',
  'project.todo': '项目任务内容',
  'project.run': '项目执行内容',
};

export function detailMarkers(value) {
  const found = [];
  const seen = new Set();
  const visit = (node) => {
    if (!node || typeof node !== 'object') return;
    if (node.omitted === true && node.read_via === 'detail_field' && typeof node.field === 'string') {
      if (!seen.has(node.field)) {
        seen.add(node.field);
        found.push(node);
      }
      return;
    }
    if (Array.isArray(node)) node.forEach(visit);
    else Object.values(node).forEach(visit);
  };
  visit(value);
  return found;
}

function payloadPath(id, marker, seq, offset) {
  if (marker.read_via === 'event_payload') {
    return `/api/executions/${encodeURIComponent(id)}/events/${seq}/payload?offset=${offset}`;
  }
  const query = new URLSearchParams({ field: marker.field, offset: String(offset) });
  return `/api/executions/${encodeURIComponent(id)}/detail-field?${query}`;
}

function markerLabel(marker) {
  if (LABELS[marker.field]) return LABELS[marker.field];
  if (marker.field?.endsWith('.session_history')) return '会话记录';
  if (marker.field?.endsWith('.result_json')) return '执行结果';
  if (marker.field?.endsWith('.last_error')) return '错误详情';
  if (marker.field?.endsWith('.plan_md')) return '计划内容';
  if (marker.field?.endsWith('.output_md')) return '执行输出';
  return '分段查看';
}

export function PayloadWindows({ id, marker, seq, label }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [windowIndex, setWindowIndex] = useState(0);
  const [windows, setWindows] = useState([{ offset: 0, leading: new Uint8Array() }]);
  const [page, setPage] = useState(null);

  useEffect(() => {
    setOpen(false); setPage(null); setError(''); setWindowIndex(0);
    setWindows([{ offset: 0, leading: new Uint8Array() }]);
  }, [id, marker.field, marker.read_via, seq]);

  const load = async (target, index) => {
    setBusy(true); setError('');
    try {
      const chunk = await apiGet(payloadPath(id, marker, seq, target.offset));
      if (!['base64', 'json-base64', 'utf8-base64'].includes(chunk.encoding) || chunk.offset !== target.offset) {
        throw new Error('分段内容格式无效');
      }
      const bytes = decodeBase64(chunk.bytes_b64 || '');
      if (chunk.next_offset !== chunk.offset + bytes.byteLength || chunk.next_offset > chunk.total_bytes) {
        throw new Error('分段内容长度无效');
      }
      const decoded = decodeWindow([bytes], target.leading, !!chunk.eof);
      setPage({
        text: decoded.text,
        start: Math.max(0, chunk.offset - target.leading.byteLength + decoded.skipped),
        end: chunk.next_offset - decoded.tail.byteLength,
        total: chunk.total_bytes,
        eof: !!chunk.eof,
        nextOffset: chunk.next_offset,
        tail: decoded.tail,
      });
      setWindowIndex(index);
    } catch (e) {
      setError(e?.message || '读取分段内容失败');
    } finally {
      setBusy(false);
    }
  };
  const show = () => { setOpen(true); load(windows[0], 0); };
  const next = () => {
    if (!page || page.eof) return;
    const target = { offset: page.nextOffset, leading: page.tail };
    const nextWindows = windows.slice(0, windowIndex + 1).concat(target);
    setWindows(nextWindows); load(target, windowIndex + 1);
  };
  const previous = () => {
    if (windowIndex === 0) return;
    load(windows[windowIndex - 1], windowIndex - 1);
  };

  if (!open) {
    return <Button size="small" onClick={show}>{label || markerLabel(marker)}</Button>;
  }
  return <div className="execution-large-field">
    <Typography.Text strong>{label || markerLabel(marker)}</Typography.Text>
    {error && <Alert type="error" showIcon title={error} />}
    {busy && !page ? <Spin size="small" /> : null}
    {page ? <>
      <Alert type="info" showIcon title="内容较大，当前按段显示" description={`当前 ${page.start}–${page.end} / ${page.total} 字节`} />
      <pre>{page.text}</pre>
      <Space>
        <Button size="small" disabled={busy || windowIndex === 0} onClick={previous}>上一段</Button>
        <Button size="small" disabled={busy || page.eof} onClick={next}>下一段</Button>
      </Space>
    </> : null}
  </div>;
}

export function InlineFields({ id, value }) {
  const markers = detailMarkers(value);
  if (!markers.length) return null;
  return <Space wrap>{markers.map((marker) => <PayloadWindows key={marker.field} id={id} marker={marker} />)}</Space>;
}

export function DetailFields({ id, detail }) {
  const markers = detailMarkers(detail).filter((marker) =>
    !marker.field.startsWith('todo.item.') && !marker.field.startsWith('project.run.'));
  if (!markers.length) return null;
  return <Space orientation="vertical" style={{ width: '100%', marginTop: 12 }}>
    <Typography.Title level={5}>较大内容</Typography.Title>
    {markers.map((marker) => <PayloadWindows key={marker.field} id={id} marker={marker} />)}
  </Space>;
}
