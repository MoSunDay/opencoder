// agentNfsCard.jsx — 「Agent 配置」页的 NFS 导出卡片：GET /api/agents/nfs
// 状态快照（running/host/port/read_only/export_root）+ Switch 显式启停
// （POST /api/agents/nfs {enabled}）。运行中给出 mount(8) 提示行；导出
// 根只读，宿主机挂载后即可浏览四类资源池。错误经 onNotice 透出服务端
// `error` 字段（apiJson 已并入）。

import { Button, Card, Descriptions, Space, Switch, Tag, Typography, message } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet, apiPost } from './api.js';
import { mountHint } from './agentsItems.js';

const { Paragraph, Text } = Typography;

export function AgentNfsCard({ onNotice }) {
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
        onNotice('获取 NFS 状态失败: ' + (e && e.message));
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
      message.success(enabled ? 'NFS 导出已启动' : 'NFS 导出已停止');
    } catch (e) {
      if (onNotice) {
        onNotice('切换 NFS 失败: ' + (e && e.message));
      }
      load(); // 与服务端实际状态对齐（失败时开关不得停在错误档位）
    } finally {
      setSwitching(false);
    }
  };

  const s = status && typeof status === 'object' ? status : {};
  return (
    <Card
      size="small"
      title="NFS 资源导出"
      loading={loading}
      extra={(
        <Space>
          <Switch checked={!!s.running} loading={switching} onChange={setEnabled} aria-label="nfs-enabled" />
          <Button size="small" onClick={load}>刷新</Button>
        </Space>
      )}
    >
      <Descriptions size="small" column={1} style={{ marginBottom: 0 }}>
        <Descriptions.Item label="状态">
          {s.running ? <Tag color="green">运行中</Tag> : <Tag>已停止</Tag>}
        </Descriptions.Item>
        <Descriptions.Item label="地址" aria-label="nfs-addr">{s.running ? `${s.host}:${s.port}` : '-'}</Descriptions.Item>
        <Descriptions.Item label="只读">{s.read_only ? '是' : '否'}</Descriptions.Item>
        <Descriptions.Item label="导出根">{s.export_root || '-'}</Descriptions.Item>
      </Descriptions>
      {s.running ? (
        <div style={{ marginTop: 8 }}>
          <Text type="secondary" style={{ fontSize: 12 }}>宿主机挂载：</Text>
          <Paragraph copyable style={{ marginBottom: 0 }}>
            <code aria-label="nfs-mount-hint">{mountHint(s)}</code>
          </Paragraph>
        </div>
      ) : null}
    </Card>
  );
}
