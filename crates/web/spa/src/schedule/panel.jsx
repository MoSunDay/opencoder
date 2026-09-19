// schedule/panel.jsx —「调度」页：控制面 cron 定时任务的管理视图。
//
// schema v27 起 libsql `schedules` 表是定义事实源，本页是它的全功能
// 前台：GET /api/schedules 给定义 + last_run / next_run；POST / PUT /
// PATCH / DELETE 走 admin CRUD（重名 409、非法 body 400、未知 id 404）；
// POST /api/schedules/:id/run 手动立即触发（绕过 enabled/overlap）；
// GET /api/schedules/:id/runs 给触发历史（Drawer）。schedules.json 已
// 降级为首次导入种子，只保留 scan_interval_secs 这个运维旋钮。页面
// 5s 轮询对齐 topics 的口径（调度扫描本身最密 15s）。

import { Alert, Button, Drawer, Popconfirm, Space, Table, Tag } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPatch, apiPost } from '../api.js';
import { useNodes } from '../fleet/useNodes.js';
import { KIND_LABELS } from '../fleet/model.js';
import { err, ok } from '../notice.js';
import { PageShell } from '../shell/pageShell.jsx';
import { MONO_VAR } from '../ui/mono.js';
import { StatusTag } from '../ui/statusTag.jsx';
import { tableLoading, tableRows } from '../ui/tableLoading.js';
import { TimeText } from '../ui/timeText.jsx';
import { ScheduleEditorModal, OVERLAP_LABELS } from './editor.jsx';

/// 触发历史 Drawer：最新 tick 在前，最多 50 条；行键是台账主键的
/// 时间半边（schedule_id 已由 Drawer 限定）。
function ScheduleRunsDrawer({ schedule, onClose, onNotice }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let live = true;
    setLoading(true);
    apiGet(`/api/schedules/${encodeURIComponent(schedule.id)}/runs?limit=50`)
      .then((j) => { if (live) setRows(j.runs || []); })
      .catch((e) => { if (live) onNotice(err(e.message)); })
      .finally(() => { if (live) setLoading(false); });
    return () => { live = false; };
  }, [schedule.id, onNotice]);
  return (
    <Drawer title={`调度 ${schedule.id} 的触发历史`} width={760} open onClose={onClose}>
      <Table
        size="small"
        scroll={{ x: 'max-content' }}
        rowKey="scheduled_for_ms"
        loading={tableLoading(loading)}
        dataSource={tableRows(loading, rows)}
        locale={{ emptyText: '暂无触发记录' }}
        columns={[
          { title: '计划时间', dataIndex: 'scheduled_for_ms', render: (v) => <TimeText ts={v} /> },
          { title: '实际触发', dataIndex: 'fired_at_ms', render: (v) => <TimeText ts={v} /> },
          { title: '状态', dataIndex: 'status', render: (v) => <StatusTag status={v} /> },
          { title: '执行 ID', dataIndex: 'execution_id', render: (v) => (v ? <span style={{ fontFamily: MONO_VAR }}>{v}</span> : '—') },
          { title: '失败原因', dataIndex: 'error', render: (v) => (v || '—') },
        ]}
      />
    </Drawer>
  );
}

export function SchedulePanel({ onNotice }) {
  const [rows, setRows] = useState([]);
  const [scanSecs, setScanSecs] = useState(null);
  const [history, setHistory] = useState(null);
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(null);
  const { nodes } = useNodes();
  /// 首屏与手动刷新遮罩表格；5s 轮询静默（对齐 topics 3s 的语义，调度
  /// 扫描本身最密 15s，5s 足够跟上下次触发时间）。
  const [loading, setLoading] = useState(true);
  const load = useCallback(async (mode = 'reset') => {
    if (mode !== 'poll') setLoading(true);
    try {
      const j = await apiGet('/api/schedules');
      setRows(j.schedules || []);
      setScanSecs(j.scan_interval_secs ?? null);
    }
    catch (e) { onNotice(err(e.message)); }
    finally { if (mode !== 'poll') setLoading(false); }
  }, [onNotice]);
  useEffect(() => {
    let live = true;
    const refresh = () => { if (live) load('poll'); };
    load('reset'); const timer = setInterval(refresh, 5000);
    return () => { live = false; clearInterval(timer); };
  }, [load]);

  const saved = async (notice, close) => {
    onNotice(notice);
    if (!close) return;
    setOpen(false);
    await load('reset');
  };

  const toggle = async (row) => {
    try {
      await apiPatch(`/api/schedules/${encodeURIComponent(row.id)}`, { enabled: !row.enabled });
      onNotice(ok(row.enabled ? '定时任务已停用' : '定时任务已启用'));
      await load('reset');
    }
    catch (e) { onNotice(err('切换启用状态失败: ' + e.message)); }
  };

  const remove = async (row) => {
    try {
      await apiDel(`/api/schedules/${encodeURIComponent(row.id)}`);
      onNotice(ok('定时任务已删除（触发历史保留）'));
      await load('reset');
    }
    catch (e) { onNotice(err('删除定时任务失败: ' + e.message)); }
  };

  const runNow = async (row) => {
    try {
      await apiPost(`/api/schedules/${encodeURIComponent(row.id)}/run`);
      onNotice(ok('已提交立即触发'));
      await load('reset');
    }
    catch (e) { onNotice(err('立即触发失败: ' + e.message)); }
  };

  return <PageShell page="schedules">
    <Alert
      type="info"
      showIcon
      style={{ marginBottom: 12 }}
      title={`定时任务存于控制面数据库（schedules.json 仅作首次导入种子）${scanSecs ? `，每 ${scanSecs} 秒扫描一次` : ''}`}
    />
    <Space style={{ marginBottom: 12 }}>
      <Button type="primary" onClick={() => { setEditing(null); setOpen(true); }}>新建任务</Button>
      <Button onClick={() => load('reset')}>刷新</Button>
    </Space>
    <Table
      size="small"
      scroll={{ x: 'max-content' }}
      rowKey="id"
      dataSource={tableRows(loading, rows)}
      loading={tableLoading(loading)}
      locale={{ emptyText: '暂无定时任务' }}
      columns={[
        { title: 'ID', dataIndex: 'id', render: (v) => <span style={{ fontFamily: MONO_VAR }}>{v}</span> },
        { title: 'cron', dataIndex: 'cron', render: (v) => <span style={{ fontFamily: MONO_VAR }}>{v}</span> },
        { title: '启用', dataIndex: 'enabled', render: (v) => <Tag color={v ? 'success' : 'default'}>{v ? '启用' : '停用'}</Tag> },
        { title: '类型', dataIndex: 'kind', render: (v) => KIND_LABELS[v] || v },
        { title: '目标', dataIndex: 'target', render: (v) => <span style={{ fontFamily: MONO_VAR }}>{v}</span> },
        { title: '重叠', dataIndex: 'overlap', render: (v) => OVERLAP_LABELS[v] || v || '—' },
        { title: '指定节点', dataIndex: 'node_id', render: (v) => (v ? <span style={{ fontFamily: MONO_VAR }}>{v}</span> : '—') },
        { title: '上次触发', dataIndex: 'last_run', render: (v) => (v ? <Space size={4}><TimeText ts={v.fired_at_ms} /><StatusTag status={v.status} /></Space> : '—') },
        { title: '下次触发', dataIndex: 'next_run', render: (v) => (v ? <TimeText ts={v} /> : '—') },
        { title: '操作', key: 'actions', render: (_, row) => <Space size={0}>
          <Button type="link" size="small" onClick={() => setHistory(row)}>触发历史</Button>
          <Popconfirm title="立即触发一次？" description="绕过启用与重叠策略，马上提交一次执行。" onConfirm={() => runNow(row)} okText="确认触发" cancelText="取消">
            <Button type="link" size="small">立即触发</Button>
          </Popconfirm>
          <Button type="link" size="small" onClick={() => { setEditing(row); setOpen(true); }}>编辑</Button>
          <Button type="link" size="small" onClick={() => toggle(row)}>{row.enabled ? '停用' : '启用'}</Button>
          <Popconfirm title="删除该定时任务？" description="定义删除后不可恢复；触发历史保留可查。" onConfirm={() => remove(row)} okText="确认删除" cancelText="取消">
            <Button danger type="link" size="small">删除</Button>
          </Popconfirm>
        </Space> },
      ]}
    />
    <ScheduleEditorModal
      open={open}
      initial={editing}
      nodes={nodes}
      onCancel={() => setOpen(false)}
      onSaved={saved}
    />
    {history && <ScheduleRunsDrawer schedule={history} onClose={() => setHistory(null)} onNotice={onNotice} />}
  </PageShell>;
}
