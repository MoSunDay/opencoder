// teamPanel.jsx — Tab 「组队」: 3s-polled team table plus the entry points to
// the four management modals (teamModals.jsx). Node pickers draw from a
// /api/nodes snapshot refreshed whenever a modal opens — the fleet tab's own
// poll is not mounted here. 能力画像 is fire-and-forget (202 accepted).

import { Button, Popconfirm, Space, Table, Tooltip } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from './api.js';
import { openTopicDetail, openTopicsForTeam } from './store.js';
import { fmtTime, teamCapSummary } from './teamItems.js';
import { CaptainModal, CreateTeamModal, MembersModal, TopicModal } from './teamModals.jsx';

const POLL_MS = 3000;

export function TeamPanel({ onNotice }) {
  const [rows, setRows] = useState([]);
  const [nodes, setNodeRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [captainTeam, setCaptainTeam] = useState(null);
  const [membersTeam, setMembersTeam] = useState(null);
  const [topicTeam, setTopicTeam] = useState(null);
  const timer = useRef(null);
  const alive = useRef(true);

  const load = useCallback(async (silent) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const j = await apiGet('/api/teams');
      if (!alive.current) {
        return;
      }
      setRows((j && j.teams) || []);
    } catch (e) {
      if (!silent && alive.current && onNotice) {
        onNotice('获取团队列表失败: ' + (e && e.message));
      }
    } finally {
      if (alive.current && !silent) {
        setLoading(false);
      }
    }
  }, [onNotice]);

  useEffect(() => {
    alive.current = true;
    load(false);
    timer.current = setInterval(() => load(true), POLL_MS);
    return () => {
      alive.current = false;
      clearInterval(timer.current);
    };
  }, [load]);

  // Picker data: one /api/nodes fetch per modal open (not polled — the fleet
  // tab already refreshes the shared snapshot when it is the active page).
  const anyModalOpen = createOpen || !!captainTeam || !!membersTeam;
  useEffect(() => {
    if (!anyModalOpen) {
      return undefined;
    }
    let ok = true;
    apiGet('/api/nodes').then((j) => {
      if (ok) {
        setNodeRows((j && j.nodes) || []);
      }
    }).catch(() => {
      // Pickers fall back to member-only options; the modal still opens.
    });
    return () => {
      ok = false;
    };
  }, [anyModalOpen]);

  const profile = async (team) => {
    try {
      await apiPost('/api/teams/' + encodeURIComponent(team.name) + '/profile', {});
      if (onNotice) {
        onNotice('能力画像任务已派发: ' + team.name);
      }
    } catch (e) {
      if (onNotice) {
        onNotice('派发画像失败: ' + ((e && e.message) || ''));
      }
    }
  };

  const columns = [
    { title: '名称', dataIndex: 'name', key: 'name', render: (v) => v || '-' },
    {
      title: '队长',
      key: 'captain',
      width: 140,
      render: (_, r) => (r.captain && (r.captain.name || r.captain.node_id)) || '-',
    },
    {
      title: '成员数',
      key: 'members',
      width: 80,
      render: (_, r) => ((r.members || []).length),
    },
    {
      title: '能力概况',
      key: 'caps',
      ellipsis: true,
      render: (_, r) => (
        <Tooltip title={teamCapSummary(r, 99)}>
          <span>{teamCapSummary(r)}</span>
        </Tooltip>
      ),
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
      width: 340,
      render: (_, r) => (
        <Space wrap>
          <Button size="small" type="link" onClick={() => setCaptainTeam(r)}>改队长</Button>
          <Button size="small" type="link" onClick={() => setMembersTeam(r)}>成员管理</Button>
          <Button size="small" type="link" onClick={() => setTopicTeam(r)}>发起话题</Button>
          <Button size="small" type="link" onClick={() => openTopicsForTeam(r.name)}>查看话题</Button>
          <Popconfirm
            title="派发能力画像？"
            description="将给每位成员派发一次能力画像任务（异步执行）。"
            okText="派发"
            cancelText="取消"
            onConfirm={() => profile(r)}
          >
            <Button size="small" type="link">能力画像</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <div>
      <div style={{ marginBottom: 12 }}>
        <Button type="primary" onClick={() => setCreateOpen(true)}>新建团队</Button>
      </div>
      <Table
        rowKey="name"
        size="middle"
        columns={columns}
        dataSource={rows}
        loading={loading}
        pagination={false}
        locale={{ emptyText: '暂无团队' }}
      />
      <CreateTeamModal
        open={createOpen}
        nodes={nodes}
        onClose={() => setCreateOpen(false)}
        onDone={() => {
          setCreateOpen(false);
          load(true);
        }}
        onNotice={onNotice}
      />
      <CaptainModal
        team={captainTeam}
        nodes={nodes}
        onClose={() => setCaptainTeam(null)}
        onDone={() => {
          setCaptainTeam(null);
          load(true);
        }}
        onNotice={onNotice}
      />
      <MembersModal
        team={membersTeam}
        nodes={nodes}
        onClose={() => setMembersTeam(null)}
        onDone={() => {
          setMembersTeam(null);
          load(true);
        }}
        onNotice={onNotice}
      />
      <TopicModal
        team={topicTeam}
        onClose={() => setTopicTeam(null)}
        onCreated={(topic) => {
          const team = topicTeam;
          setTopicTeam(null);
          load(true);
          if (team && topic && topic.topic_id) {
            openTopicDetail(team.name, topic.topic_id);
          } else if (team) {
            openTopicsForTeam(team.name);
          }
        }}
        onNotice={onNotice}
      />
    </div>
  );
}
