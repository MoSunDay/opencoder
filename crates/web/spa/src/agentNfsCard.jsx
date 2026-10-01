// agentNfsCard.jsx — 「Agent 配置」页的 NFS 导出卡片：GET /api/agents/nfs
// 状态快照（running/host/port/read_only/export_root）+ Switch 显式启停
// （POST /api/agents/nfs {enabled}）。运行中给出 mount(8) 提示行；导出
// 根只读，宿主机挂载后即可浏览四类资源池。错误经 onNotice 透出服务端
// `error` 字段（apiJson 已并入）。

import { Alert, Button, Descriptions, Modal, Space, Switch, Tag, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from './api.js';
import { mountHint } from './agentsItems.js';
import { err } from './notice.js';
import { useMessage } from './ui/appMessage.js';
import { MONO_VAR } from './ui/mono.js';

const { Paragraph, Text } = Typography;

function readStatus(value, root) {
  if (typeof value?.status?.running !== 'boolean') throw new Error('NFS 状态格式错误');
  return { ...value.status, export_root: value.root || value.status.export_root || root };
}

export function AgentNfsCard({ onNotice, endpoint = '/api/agents/nfs', title = 'NFS 资源导出', label = 'nfs' }) {
  const msg = useMessage();
  const [status, setStatus] = useState(null);
  const [loading, setLoading] = useState(true);
  const [switching, setSwitching] = useState(false);
  const [error, setError] = useState('');
  const [confirmStop, setConfirmStop] = useState(false);
  const request = useRef(null);

  const load = useCallback(async () => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    setLoading(true);
    try {
      const j = await apiGet(endpoint, { signal: controller.signal });
      if (controller.signal.aborted) return;
      setStatus(readStatus(j));
      setError('');
    } catch (e) {
      if (controller.signal.aborted) return;
      setError(e.message);
      if (onNotice) {
        onNotice(err('获取 NFS 状态失败: ' + (e && e.message)));
      }
    } finally {
      if (!controller.signal.aborted) setLoading(false);
    }
  }, [onNotice, endpoint]);

  useEffect(() => {
    setStatus(null);
    load();
    return () => request.current?.abort();
  }, [load]);

  const setEnabled = async (enabled) => {
    if (switching) return;
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    setSwitching(true);
    try {
      const j = await apiPost(endpoint, { enabled }, { signal: controller.signal });
      if (controller.signal.aborted) return;
      setStatus(readStatus(j, status?.export_root));
      setError(''); setConfirmStop(false);
      msg.success(enabled ? 'NFS 导出已启动' : 'NFS 导出已停止');
    } catch (e) {
      if (controller.signal.aborted) return;
      setError(e.message);
      if (onNotice) {
        onNotice(err('切换 NFS 失败: ' + (e && e.message)));
      }
      setSwitching(false);
      load();
    } finally {
      if (!controller.signal.aborted) setSwitching(false);
    }
  };

  const s = status && typeof status === 'object' ? status : {};
  return (
    <div style={{ marginTop: 16 }}>
      <Space style={{ marginBottom: 8, justifyContent: 'space-between', width: '100%' }}>
        <Space>
          <Typography.Title level={5} style={{ margin: 0 }}>{title}</Typography.Title>
          {status && !error ? (s.running ? <Tag color="green">运行中</Tag> : <Tag>已停止</Tag>) : <Tag>状态未知</Tag>}
        </Space>
        <Space>
          <Switch checked={!!s.running} loading={switching} disabled={loading || !!error || !status} onChange={(enabled) => enabled ? setEnabled(true) : setConfirmStop(true)} aria-label={`${label}-enabled`} />
          <Button size="small" disabled={switching} onClick={load}>刷新</Button>
        </Space>
      </Space>
      {error && <Alert type="error" showIcon title={error} action={<Button onClick={load}>重试导出状态</Button>} />}
      {loading ? <Text type="secondary">加载中…</Text> : (
        <>
          {/* antd 6 只把 items 里的已知键（label/children/span/…）投给单元格，
              多余 props 会被丢掉 —— aria-label 只能挂在内容上（span 包一层）。 */}
          <Descriptions size="small" column={4} bordered items={[
            { key: 'addr', label: '地址', children: <span aria-label={`${label}-addr`}>{s.running ? `${s.host}:${s.port}` : '-'}</span> },
            { key: 'read_only', label: '只读', children: status && !error ? (s.read_only ? '是' : '否') : '—' },
            { key: 'export_root', label: '导出根', span: 2, children: s.export_root || '-' },
          ]} />
          {s.running ? (
            <div style={{ marginTop: 8 }}>
              <Text type="secondary" style={{ fontSize: 12 }}>宿主机挂载：</Text>
              <Paragraph copyable style={{ marginBottom: 0 }}>
                <code aria-label={`${label}-mount-hint`} style={{ fontFamily: MONO_VAR }}>{mountHint(s)}</code>
              </Paragraph>
            </div>
          ) : null}
        </>
      )}
      <Modal open={confirmStop} title={`停止${title}？`} okText="确认停止导出" cancelText="取消" confirmLoading={switching}
        onOk={() => setEnabled(false)} onCancel={() => { if (!switching) setConfirmStop(false); }}>
        停止导出可能影响正在读取资源或工作区的任务，以及后续任务的受理。不会修改源目录或已保存的执行记录。
      </Modal>
    </div>
  );
}
