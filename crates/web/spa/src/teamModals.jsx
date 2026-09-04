// teamModals.jsx — the four team management modals behind the 组队 tab.
// Same shape as modelModal.jsx / questionModal.jsx: footer-button Modal,
// local form state reset on (re)open, busy flag while the request runs,
// failures surfaced through onNotice. `team` doubles as the open flag —
// null means closed, so the parent only tracks "which team".
//   CreateTeamModal  POST   /api/teams                        {name, captain_node_id, member_node_ids}
//   CaptainModal     PATCH  /api/teams/:name                  {captain_node_id}
//   MembersModal     POST   /api/teams/:name/members          {add, remove}
//   TopicModal       POST   /api/teams/:name/topics           {title, requirement}

import { Button, Input, Modal, Radio, Select, Space, Tag, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiPatch, apiPost } from './api.js';
import { captainOptions, memberCapsText, nodeSelectOptions } from './teamItems.js';

const { Text } = Typography;

export function CreateTeamModal({ open, nodes, onClose, onDone, onNotice }) {
  const [name, setName] = useState('');
  const [captain, setCaptain] = useState('');
  const [members, setMembers] = useState([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setName('');
      setCaptain('');
      setMembers([]);
    }
  }, [open]);

  const submit = async () => {
    if (!name.trim() || !captain) {
      return;
    }
    setBusy(true);
    try {
      await apiPost('/api/teams', {
        name: name.trim(),
        captain_node_id: captain,
        member_node_ids: members,
      });
      if (onNotice) {
        onNotice('团队已创建: ' + name.trim());
      }
      onDone();
    } catch (e) {
      if (onNotice) {
        onNotice('创建团队失败: ' + ((e && e.message) || ''));
      }
    }
    setBusy(false);
  };

  const opts = nodeSelectOptions(nodes);
  return (
    <Modal
      title="新建团队"
      open={open}
      onCancel={onClose}
      footer={[
        <Button key="cancel" onClick={onClose}>取消</Button>,
        <Button key="ok" type="primary" disabled={busy || !name.trim() || !captain} onClick={submit}>创建</Button>,
      ]}
    >
      <Space direction="vertical" size={12} style={{ width: '100%' }}>
        <Input placeholder="团队名称" value={name} onChange={(e) => setName(e.target.value)} />
        <div>
          <Text type="secondary">队长（单选）</Text>
          <Radio.Group
            value={captain}
            onChange={(e) => setCaptain(e.target.value)}
            style={{ display: 'block', marginTop: 4 }}
          >
            {opts.map((o) => (
              <Radio key={o.value} value={o.value} style={{ display: 'block', padding: '2px 0' }}>{o.label}</Radio>
            ))}
          </Radio.Group>
        </div>
        <div>
          <Text type="secondary">成员（多选）</Text>
          <Select
            mode="multiple"
            style={{ width: '100%', marginTop: 4 }}
            placeholder="选择成员节点"
            value={members}
            onChange={setMembers}
            options={opts}
            showSearch
            optionFilterProp="label"
          />
        </div>
      </Space>
    </Modal>
  );
}

export function CaptainModal({ team, nodes, onClose, onDone, onNotice }) {
  const [sel, setSel] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setSel('');
  }, [team]);

  const submit = async () => {
    if (!team || !sel) {
      return;
    }
    setBusy(true);
    try {
      await apiPatch('/api/teams/' + encodeURIComponent(team.name), { captain_node_id: sel });
      if (onNotice) {
        onNotice('队长已更新: ' + team.name);
      }
      onDone();
    } catch (e) {
      if (onNotice) {
        onNotice('更新队长失败: ' + ((e && e.message) || ''));
      }
    }
    setBusy(false);
  };

  return (
    <Modal
      title={'改队长 · ' + ((team && team.name) || '')}
      open={!!team}
      onCancel={onClose}
      footer={[
        <Button key="cancel" onClick={onClose}>取消</Button>,
        <Button key="ok" type="primary" disabled={busy || !sel} onClick={submit}>确定</Button>,
      ]}
    >
      <Radio.Group value={sel} onChange={(e) => setSel(e.target.value)}>
        {captainOptions(team, nodes).map((o) => (
          <Radio key={o.value} value={o.value} style={{ display: 'block', padding: '2px 0' }}>{o.label}</Radio>
        ))}
      </Radio.Group>
    </Modal>
  );
}

export function MembersModal({ team, nodes, onClose, onDone, onNotice }) {
  const [adds, setAdds] = useState([]);
  const [removes, setRemoves] = useState([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setAdds([]);
    setRemoves([]);
  }, [team]);

  const members = (team && Array.isArray(team.members)) ? team.members : [];
  const currentIds = members.map((m) => m.node_id);
  const toggleRemove = (id) => {
    setRemoves((rs) => (rs.includes(id) ? rs.filter((x) => x !== id) : rs.concat(id)));
  };
  const addOptions = nodeSelectOptions(nodes).filter((o) => !currentIds.includes(o.value));

  const submit = async () => {
    if (!team || (adds.length === 0 && removes.length === 0)) {
      return;
    }
    setBusy(true);
    try {
      await apiPost('/api/teams/' + encodeURIComponent(team.name) + '/members', { add: adds, remove: removes });
      if (onNotice) {
        onNotice('成员已更新: ' + team.name);
      }
      onDone();
    } catch (e) {
      if (onNotice) {
        onNotice('更新成员失败: ' + ((e && e.message) || ''));
      }
    }
    setBusy(false);
  };

  return (
    <Modal
      title={'成员管理 · ' + ((team && team.name) || '')}
      open={!!team}
      onCancel={onClose}
      footer={[
        <Button key="cancel" onClick={onClose}>取消</Button>,
        <Button key="ok" type="primary" disabled={busy || (adds.length === 0 && removes.length === 0)} onClick={submit}>确定</Button>,
      ]}
    >
      <Space direction="vertical" size={12} style={{ width: '100%' }}>
        <div>
          <Text type="secondary">当前成员</Text>
          <div style={{ marginTop: 4 }}>
            {members.length === 0 ? <Text type="secondary">暂无成员</Text> : members.map((m) => (
              <div key={m.node_id} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '2px 0' }}>
                <Tag color={removes.includes(m.node_id) ? 'red' : 'blue'}>{m.name || m.node_id}</Tag>
                <Text type="secondary" style={{ fontSize: 12 }}>{memberCapsText(m)}</Text>
                <Button size="small" type="link" danger onClick={() => toggleRemove(m.node_id)}>
                  {removes.includes(m.node_id) ? '撤销退出' : '退出'}
                </Button>
              </div>
            ))}
          </div>
        </div>
        <div>
          <Text type="secondary">添加成员</Text>
          <Select
            mode="multiple"
            style={{ width: '100%', marginTop: 4 }}
            placeholder="选择要加入的节点"
            value={adds}
            onChange={setAdds}
            options={addOptions}
            showSearch
            optionFilterProp="label"
          />
        </div>
      </Space>
    </Modal>
  );
}

export function TopicModal({ team, onClose, onCreated, onNotice }) {
  const [title, setTitle] = useState('');
  const [requirement, setRequirement] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setTitle('');
    setRequirement('');
  }, [team]);

  const submit = async () => {
    if (!team || !title.trim()) {
      return;
    }
    setBusy(true);
    try {
      const j = await apiPost('/api/teams/' + encodeURIComponent(team.name) + '/topics', {
        title: title.trim(),
        requirement: requirement.trim(),
      });
      if (onNotice) {
        onNotice('话题已创建: ' + title.trim());
      }
      onCreated((j && j.topic) || {});
    } catch (e) {
      if (onNotice) {
        onNotice('创建话题失败: ' + ((e && e.message) || ''));
      }
    }
    setBusy(false);
  };

  return (
    <Modal
      title={'发起话题 · ' + ((team && team.name) || '')}
      open={!!team}
      onCancel={onClose}
      footer={[
        <Button key="cancel" onClick={onClose}>取消</Button>,
        <Button key="ok" type="primary" disabled={busy || !title.trim()} onClick={submit}>创建</Button>,
      ]}
    >
      <Space direction="vertical" size={12} style={{ width: '100%' }}>
        <Input placeholder="话题标题" value={title} onChange={(e) => setTitle(e.target.value)} />
        <Input.TextArea
          rows={4}
          placeholder="需求描述 (requirement)"
          value={requirement}
          onChange={(e) => setRequirement(e.target.value)}
        />
      </Space>
    </Modal>
  );
}
