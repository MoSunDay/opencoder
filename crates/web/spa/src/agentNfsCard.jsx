// agentNfsCard.jsx — 「Agent 配置」页的 NFS 导出卡片：GET /api/agents/nfs
// 状态快照（running/host/port/read_only/export_root）+ Switch 显式启停
// （POST /api/agents/nfs {enabled}）。运行中给出 mount(8) 提示行；导出
// 根只读，宿主机挂载后即可浏览四类资源池。错误经 onNotice 透出服务端
// `error` 字段（apiJson 已并入）。

import { Button, Descriptions, Space, Switch, Tag, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet, apiPost } from './api.js';
import { mountHint } from './agentsItems.js';
import { err } from './notice.js';
import { useMessage } from './ui/appMessage.js';

const { Paragraph, Text } = Typography;

export function AgentNfsCard({ onNotice }) {
  const msg = useMessage();
  const [status, setStatus] = useState(null);
  const [loading, setLoading] = useState(true);
  const [switching, setSwitching] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const j = await apiGet('/api/agents/nfs');
      setStatus((j && j.status) || null);
    } catch (e) {
      if (onNotice) {
        onNotice(err('获取 NFS 状态失败: ' + (e && e.message)));
      }
    } finally {
      setLoading(false);
    }
  }, [onNotice]);

  useEffect(() => {
    load();
  }, [load]);

  const setEnabled = async (enabled) => {
    setSwitching(true);
    try {
      const j = await apiPost('/api/agents/nfs', { enabled });
      setStatus((j && j.status) || null);
      msg.success(enabled ? 'NFS 导出已启动' : 'NFS 导出已停止');
    } catch (e) {
      if (onNotice) {
        onNotice(err('切换 NFS 失败: ' + (e && e.message)));
      }
      load(); // 与服务端实际状态对齐（失败时开关不得停在错误档位）
    } finally {
      setSwitching(false);
    }
  };

  const s = status && typeof status === 'object' ? status : {};
  return (
    <div style={{ marginTop: 16 }}>
      <Space style={{ marginBottom: 8, justifyContent: 'space-between', width: '100%' }}>
        <Space>
          <Typography.Title level={5} style={{ margin: 0 }}>NFS 资源导出</Typography.Title>
          {s.running ? <Tag color="green">运行中</Tag> : <Tag>已停止</Tag>}
        </Space>
        <Space>
          <Switch checked={!!s.running} loading={switching} onChange={setEnabled} aria-label="nfs-enabled" />
          <Button size="small" onClick={load}>刷新</Button>
        </Space>
      </Space>
      {loading ? <Text type="secondary">加载中…</Text> : (
        <>
          {/* antd 6 只把 items 里的已知键（label/children/span/…）投给单元格，
              多余 props 会被丢掉 —— aria-label 只能挂在内容上（span 包一层）。 */}
          <Descriptions size="small" column={4} bordered items={[
            { key: 'addr', label: '地址', children: <span aria-label="nfs-addr">{s.running ? `${s.host}:${s.port}` : '-'}</span> },
            { key: 'read_only', label: '只读', children: s.read_only ? '是' : '否' },
            { key: 'export_root', label: '导出根', span: 2, children: s.export_root || '-' },
          ]} />
          {s.running ? (
            <div style={{ marginTop: 8 }}>
              <Text type="secondary" style={{ fontSize: 12 }}>宿主机挂载：</Text>
              <Paragraph copyable style={{ marginBottom: 0 }}>
                <code aria-label="nfs-mount-hint">{mountHint(s)}</code>
              </Paragraph>
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}
