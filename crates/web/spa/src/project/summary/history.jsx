import { Alert, Button, Card, Drawer, Popconfirm, Space, Spin, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import dayjs from 'dayjs';
import { apiGet, apiPost } from '../../api.js';
import { EditButton } from '../../ui/permissions.jsx';
import { Markdown } from '../markdown.jsx';

export function SummaryHistory({ collection, record, refresh, compact = false }) {
  const [open, setOpen] = useState(false);
  const [data, setData] = useState(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(null);
  const [reload, setReload] = useState(0);
  const restoring = useRef(null);
  const path = `/api/project/${collection}/${encodeURIComponent(record.id)}/summary-versions`;

  useEffect(() => () => restoring.current?.abort(), []);
  useEffect(() => {
    if (!open) return;
    const controller = new AbortController();
    setLoading(true); setError(''); setData(null);
    apiGet(path, { signal: controller.signal }).then((value) => {
      if (!controller.signal.aborted) setData({ path, versions: value.versions });
    }).catch((failure) => {
      if (!controller.signal.aborted) setError(failure.message);
    }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [open, path, record.summary_updated_at, reload]);

  const restore = async (version) => {
    if (restoring.current) return;
    const controller = new AbortController();
    restoring.current = controller;
    setSaving(version); setError('');
    try {
      await apiPost(`${path}/${version}/restore`, {}, { signal: controller.signal });
      if (!controller.signal.aborted) {
        await refresh?.();
        if (!controller.signal.aborted) setReload((value) => value + 1);
      }
    } catch (failure) {
      if (!controller.signal.aborted) setError(failure.message);
    } finally {
      restoring.current = null;
      if (!controller.signal.aborted) setSaving(null);
    }
  };
  const versions = data?.path === path ? data.versions : [];
  return <span onClick={(event) => event.stopPropagation()}>
    <Button size="small" type={compact ? 'link' : 'default'} onClick={() => setOpen(true)}>{compact ? '概述历史' : '修改记录'}</Button>
    <Drawer open={open} title="概述修改记录" size="min(720px, 100vw)" onClose={() => { if (saving == null) setOpen(false); }} destroyOnHidden>
      <Space orientation="vertical" size={16} style={{ width: '100%' }}>
        <Typography.Text type="secondary">保留最近 10 个版本（含当前版本）。回滚会新增一个版本。</Typography.Text>
        {error && <Alert type="error" title={error} action={<Button onClick={() => setReload((value) => value + 1)}>重试</Button>} />}
        {loading ? <Spin aria-label="正在加载修改记录" /> : versions.map((version, index) => <Card key={version.version} size="small"
          title={<Space wrap><span>版本 {version.version}{index === 0 ? '（当前）' : ''}</span><Typography.Text type="secondary">{dayjs(version.created_at).format('YYYY-MM-DD HH:mm:ss')}</Typography.Text></Space>}
          extra={index > 0 && <Popconfirm title={`恢复到版本 ${version.version}？`} description="当前内容会被替换，并保存为新的版本。" onConfirm={() => restore(version.version)} okText="确认恢复" cancelText="取消" disabled={saving != null}>
            <EditButton size="small" disabled={saving != null} loading={saving === version.version}>恢复此版本</EditButton>
          </Popconfirm>}>
          {version.restored_from != null && <Typography.Paragraph type="secondary">恢复自版本 {version.restored_from}</Typography.Paragraph>}
          {version.current_summary ? <Markdown text={version.current_summary} /> : <Typography.Text type="secondary">空概述</Typography.Text>}
        </Card>)}
        {!loading && !error && data && versions.length === 0 && <Typography.Text type="secondary">暂无修改记录</Typography.Text>}
      </Space>
    </Drawer>
  </span>;
}
