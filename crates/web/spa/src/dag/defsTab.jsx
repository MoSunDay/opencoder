// defsTab.jsx — DAG「定义」tab: definitions table (name / step count /
// updated_at / actions) + dispatch modal (optional target node from the
// shared fleet snapshot) + create/edit drawer (defEditor.jsx).
// Endpoints: GET /api/dag/defs, POST /api/dag/defs, DELETE /api/dag/defs/:id,
// POST /api/dag/defs/:id/dispatch {node_id?} → {run_id}.

import { Button, message, Modal, Popconfirm, Select, Space, Table, Tag, Tooltip, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPost } from '../api.js';
import { TimeText } from '../ui/timeText.jsx';
import { useStore } from '../store.js';
import { newId, nodeOptions as buildNodeOptions } from '../fleet/model.js';
import { DefEditor } from './defEditor.jsx';
import { err } from '../notice.js';

const { Text } = Typography;

const KIND_COLOR = { agent: 'geekblue', wasm: 'green', runner: 'purple' };

/// Step-kind mini tags for the 步骤 column (first kinds, then "+n").
function KindSummary({ spec }) {
  const kinds = ((spec && spec.steps) || []).map((s) => (s.kind && s.kind.type) || '?');
  const head = kinds.slice(0, 3);
  return (
    <Space size={4} wrap>
      {head.map((k, i) => (
        <Tag key={i} color={KIND_COLOR[k] || 'default'} style={{ marginInlineEnd: 0 }}>
          {k}
        </Tag>
      ))}
      {kinds.length > head.length ? <Text type="secondary">+{kinds.length - head.length}</Text> : null}
    </Space>
  );
}

export function DefsTab({ onNotice, onDispatched }) {
  const { nodes } = useStore();
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [editorOpen, setEditorOpen] = useState(false);
  const [editing, setEditing] = useState(null); // def being edited, null = create
  const [saving, setSaving] = useState(false);
  const [dispatchFor, setDispatchFor] = useState(null); // def row in dispatch modal
  const [dispatchNode, setDispatchNode] = useState(undefined);
  const [dispatching, setDispatching] = useState(false);
  const alive = useRef(true);
  const attempt = useRef(null);

  const load = useCallback(
    async (silent) => {
      if (!silent) {
        setLoading(true);
      }
      try {
        const list = await apiGet('/api/dag/defs');
        if (!alive.current) {
          return;
        }
        setRows(Array.isArray(list) ? list : []);
      } catch (e) {
        if (!silent && alive.current && onNotice) {
          onNotice(err('获取工作流定义失败: ' + (e && e.message)));
        }
      } finally {
        if (alive.current && !silent) {
          setLoading(false);
        }
      }
    },
    [onNotice],
  );

  useEffect(() => {
    alive.current = true;
    load(false);
    return () => {
      alive.current = false;
    };
  }, [load]);

  const save = async (spec) => {
    setSaving(true);
    try {
      await apiPost('/api/dag/defs', { spec });
    } finally {
      if (alive.current) {
        setSaving(false);
      }
    }
    await load(true);
    setEditorOpen(false);
  };

  const remove = async (id) => {
    try {
      await apiDel('/api/dag/defs/' + encodeURIComponent(id));
      await load(true);
    } catch (e) {
      if (onNotice) {
        onNotice(err('删除定义失败: ' + (e && e.message)));
      }
    }
  };

  const dispatch = async () => {
    const def = dispatchFor;
    if (!def) {
      return;
    }
    const key = JSON.stringify([def.id, dispatchNode]);
    if (attempt.current?.key !== key) attempt.current = { key, id: newId('dag') };
    setDispatching(true);
    try {
      const j = await apiPost(
        '/api/dag/defs/' + encodeURIComponent(def.id) + '/dispatch',
        { id: attempt.current.id, ...(dispatchNode ? { node_id: dispatchNode } : {}) },
      );
      const runId = j && j.run_id ? j.run_id : '';
      if (onNotice) {
        onNotice(err(''));
      }
      message.success('已派发，运行 ID: ' + (runId ? runId.slice(0, 8) : '(unknown)'));
      attempt.current = null;
      setDispatchFor(null);
      if (onDispatched) {
        onDispatched(runId);
      }
    } catch (e) {
      if (onNotice) {
        onNotice(err('派发失败: ' + (e && e.message)));
      }
    } finally {
      if (alive.current) {
        setDispatching(false);
      }
    }
  };

  const columns = [
    {
      title: '名称',
      dataIndex: 'name',
      key: 'name',
      render: (v, r) => (
        <Space direction="vertical" size={0}>
          <Text strong>{v || r.id}</Text>
          {r.error ? <Tag color="warning" style={{ marginTop: 2 }}>定义无法解析：{r.error}</Tag> : null}
          {r.spec && r.spec.description ? <Text type="secondary" style={{ fontSize: 12 }}>{r.spec.description}</Text> : null}
        </Space>
      ),
    },
    {
      title: '步骤数',
      key: 'steps',
      width: 90,
      align: 'center',
      render: (_, r) => ((r.spec && r.spec.steps) || []).length,
    },
    {
      title: '类型',
      key: 'kinds',
      width: 170,
      render: (_, r) => <KindSummary spec={r.spec} />,
    },
    {
      title: '更新时间',
      dataIndex: 'updated_at',
      key: 'updated_at',
      width: 130,
      render: (ts) => <TimeText ts={ts} />,
    },
    {
      title: '操作',
      key: 'ops',
      width: 230,
      render: (_, r) => (
        <Space>
          <Button
            size="small"
            type="link"
            disabled={!!r.error}
            onClick={() => { setDispatchNode(undefined); setDispatchFor(r); }}
          >
            派发
          </Button>
          <Button
            size="small"
            type="link"
            disabled={!!r.error}
            onClick={() => {
              setEditing(r);
              setEditorOpen(true);
            }}
          >
            编辑
          </Button>
          <Popconfirm
            title="删除该定义？"
            description="不影响已派发的运行（运行持有 spec 快照）。"
            okText="删除"
            okButtonProps={{ danger: true }}
            cancelText="取消"
            onConfirm={() => remove(r.id)}
          >
            <Button size="small" type="link" danger>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  const dispatchNodeOptions = buildNodeOptions(nodes || [], 'dag');

  return (
    <Space direction="vertical" size={12} style={{ width: '100%' }}>
      <Space>
        <Button
          type="primary"
          onClick={() => {
            setEditing(null);
            setEditorOpen(true);
          }}
        >
          新建定义
        </Button>
        <Button onClick={() => load(false)}>刷新</Button>
      </Space>
      <Table
        rowKey="id"
        size="middle"
        columns={columns}
        dataSource={rows}
        loading={loading}
        pagination={false}
        locale={{ emptyText: '暂无工作流定义' }}
      />
      <DefEditor
        open={editorOpen}
        def={editing}
        saving={saving}
        onClose={() => setEditorOpen(false)}
        onSave={save}
      />
      <Modal
        title={'派发「' + ((dispatchFor && dispatchFor.name) || '') + '」'}
        open={!!dispatchFor}
        onOk={dispatch}
        onCancel={() => setDispatchFor(null)}
        okText="确认派发"
        cancelText="取消"
        confirmLoading={dispatching}
      >
        <Space direction="vertical" size={8} style={{ width: '100%' }}>
          <Text type="secondary">整个工作流会在同一个节点完成。留空时由服务端选择当前可用节点。</Text>
          <Select
            style={{ width: '100%' }}
            allowClear
            placeholder="任意节点（默认）"
            value={dispatchNode}
            onChange={setDispatchNode}
            options={dispatchNodeOptions}
            notFoundContent="暂无可用 DAG 节点"
          />
        </Space>
      </Modal>
    </Space>
  );
}
