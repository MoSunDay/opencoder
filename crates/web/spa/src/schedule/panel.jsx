// schedule/panel.jsx —「调度」页：控制面 cron 定时任务的只读视图。
//
// schedules.json（控制面工作目录的领域文件）是唯一定义事实源，DB 只记
// 触发台账，所以这里没有编辑器：GET /api/schedules 给定义 + last_run /
// next_run，GET /api/schedules/:id/runs 给触发历史（Drawer）。两端点均
// admin-only，导航侧非 admin 只见 topics，口径天然一致。invalid cron 的
// 条目 next_run 为 null，渲染「—」——定义有错去改文件，不在页面上修。

import { Alert, Button, Drawer, Space, Table, Tag } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet } from '../api.js';
import { KIND_LABELS } from '../fleet/model.js';
import { err } from '../notice.js';
import { PageShell } from '../shell/pageShell.jsx';
import { MONO_VAR } from '../ui/mono.js';
import { StatusTag } from '../ui/statusTag.jsx';
import { tableLoading, tableRows } from '../ui/tableLoading.js';
import { TimeText } from '../ui/timeText.jsx';

/// overlap 序列化为 `skip` | `allow`（crates/core config/schedule.rs）。
const OVERLAP_LABELS = { skip: '跳过重叠', allow: '允许重叠' };

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
        dataSource={tableRows(loading, rows)}
        loading={tableLoading(loading)}
        locale={{ emptyText: '暂无触发记录' }}
        columns={[
          { title: '计划时间', dataIndex: 'scheduled_for_ms', render: (v) => <TimeText ts={v} /> },
          { title: '实际触发', dataIndex: 'fired_at_ms', render: (v) => <TimeText ts={v} /> },
          { title: '状态', dataIndex: 'status', render: (v) => <StatusTag status={v} /> },
          { title: '执行 ID', dataIndex: 'execution_id', render: (v) => (v ? <span style={{ fontFamily: MONO_VAR }}>{v}</span> : '—') },
          { title: '失败原因', dataIndex: 'error', render: (v) => v || '—' },
        ]}
      />
    </Drawer>
  );
}

export function SchedulePanel({ onNotice }) {
  const [rows, setRows] = useState([]);
  const [scanSecs, setScanSecs] = useState(null);
  const [history, setHistory] = useState(null);
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
  return <PageShell page="schedules">
    <Alert
      type="info"
      showIcon
      style={{ marginBottom: 12 }}
      title={`定时任务由控制面 schedules.json 定义，本页只读${scanSecs ? `（每 ${scanSecs} 秒扫描一次）` : ''}`}
    />
    <Space style={{ marginBottom: 12 }}><Button onClick={() => load('reset')}>刷新</Button></Space>
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
        { title: '操作', key: 'actions', render: (_, row) => <Button type="link" onClick={() => setHistory(row)}>触发历史</Button> },
      ]}
    />
    {history && <ScheduleRunsDrawer schedule={history} onClose={() => setHistory(null)} onNotice={onNotice} />}
  </PageShell>;
}
