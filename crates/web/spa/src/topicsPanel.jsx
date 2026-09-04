// topicsPanel.jsx — Tab 「话题」: team-filtered topic table with 3s polling.
// The filter rides the store (topicsTeamFilter) so the 组队 tab's 查看话题
// can pre-filter this list via openTopicsForTeam; the Select writes the same
// field, keeping one source of truth. cancel/resume are request-then-refresh
// — the 3s poll converges the status Tag either way.

import { Button, Popconfirm, Select, Space, Table, Tag, Tooltip, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from './api.js';
import { openTopicDetail, setTopicsTeamFilter, useStore } from './store.js';
import { finishReasonText, fmtTime, topicCancelable, topicResumable, topicStatusView } from './teamItems.js';

const POLL_MS = 3000;

function StatusTag({ topic }) {
  const v = topicStatusView(topic);
  return <Tag color={v.color}>{v.label}</Tag>;
}

function topicPath(t, action) {
  return '/api/teams/' + encodeURIComponent(t.team_name) + '/topics/' + encodeURIComponent(t.topic_id) + '/' + action;
}

export function TopicsPanel({ onNotice }) {
  const { topicsTeamFilter } = useStore();
  const [rows, setRows] = useState([]);
  const [teams, setTeams] = useState([]);
  const [loading, setLoading] = useState(false);
  const timer = useRef(null);
  const alive = useRef(true);

  const load = useCallback(async (silent) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const path = topicsTeamFilter
        ? '/api/topics?team=' + encodeURIComponent(topicsTeamFilter)
        : '/api/topics';
      const [t, g] = await Promise.all([
        apiGet(path),
        apiGet('/api/teams').catch(() => null), // filter options degrade silently
      ]);
      if (!alive.current) {
        return;
      }
      setRows((t && t.topics) || []);
      if (g && Array.isArray(g.teams)) {
        setTeams(g.teams);
      }
    } catch (e) {
      if (!silent && alive.current && onNotice) {
        onNotice('获取话题列表失败: ' + (e && e.message));
      }
    } finally {
      if (alive.current && !silent) {
        setLoading(false);
      }
    }
  }, [onNotice, topicsTeamFilter]);

  useEffect(() => {
    alive.current = true;
    load(false);
    timer.current = setInterval(() => load(true), POLL_MS);
    return () => {
      alive.current = false;
      clearInterval(timer.current);
    };
  }, [load]);

  const act = async (topic, action, okText) => {
    try {
      await apiPost(topicPath(topic, action), {});
      if (onNotice) {
        onNotice(okText + ': ' + (topic.title || topic.topic_id));
      }
    } catch (e) {
      if (onNotice) {
        onNotice(okText + '失败: ' + ((e && e.message) || ''));
      }
    }
    load(true);
  };

  const columns = [
    { title: '标题', dataIndex: 'title', key: 'title', ellipsis: true, render: (v) => v || '-' },
    { title: '所属团队', dataIndex: 'team_name', key: 'team_name', width: 130, render: (v) => v || '-' },
    { title: '状态', key: 'status', width: 100, render: (_, r) => <StatusTag topic={r} /> },
    {
      title: '结束原因',
      dataIndex: 'finish_reason',
      key: 'finish_reason',
      width: 110,
      render: (v) => finishReasonText(v),
    },
    {
      title: '创建时间',
      dataIndex: 'created_at',
      key: 'created_at',
      width: 170,
      render: (ts) => (
        <Tooltip title={fmtTime(ts)}>
          <span>{fmtTime(ts)}</span>
        </Tooltip>
      ),
    },
    {
      title: '操作',
      key: 'ops',
      width: 180,
      render: (_, r) => (
        <Space>
          <Button size="small" type="link" onClick={() => openTopicDetail(r.team_name, r.topic_id)}>详情</Button>
          {topicCancelable(r) ? (
            <Popconfirm
              title="取消该话题？"
              description="将终止后续轮次，成员停止汇报。"
              okText="取消话题"
              okButtonProps={{ danger: true }}
              cancelText="返回"
              onConfirm={() => act(r, 'cancel', '已请求取消')}
            >
              <Button size="small" type="link" danger>取消</Button>
            </Popconfirm>
          ) : null}
          {topicResumable(r) ? (
            <Button size="small" type="link" onClick={() => act(r, 'resume', '已请求恢复')}>恢复</Button>
          ) : null}
        </Space>
      ),
    },
  ];

  return (
    <div>
      <div style={{ marginBottom: 12, display: 'flex', gap: 8, alignItems: 'center' }}>
        <Typography.Text type="secondary">团队过滤</Typography.Text>
        <Select
          style={{ width: 240 }}
          allowClear
          placeholder="全部团队"
          value={topicsTeamFilter || undefined}
          onChange={(v) => setTopicsTeamFilter(v || null)}
          options={teams.map((t) => ({ value: t.name, label: t.name }))}
          showSearch
          optionFilterProp="label"
        />
      </div>
      <Table
        rowKey="topic_id"
        size="middle"
        columns={columns}
        dataSource={rows}
        loading={loading}
        pagination={false}
        locale={{ emptyText: '暂无话题' }}
      />
    </div>
  );
}
