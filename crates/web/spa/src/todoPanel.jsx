// todoPanel.jsx — 菜单页「TODO 管理」: 两个 tab。
//   模板 — templates 表 + 展开行版本列表（编辑/设为当前/新版本/删除版本/
//           运行）+ 新建模板内联表单（预填最小合法 spec 示例）。
//   运行 — todoRunsPanel.jsx 的工作流列表 + 事件流。
// 版本行上的「运行」成功后带 workflow_id 跳到「运行」tab 并聚焦该工作流。

import { Button, Card, Col, Form, Input, Popconfirm, Row, Space, Table, Tabs, Tag, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPost, apiPut } from './api.js';
import { TodoEditor } from './todoEditor.jsx';
import { TodoRunsPanel } from './todoRunsPanel.jsx';

const { TextArea } = Input;
const { Text } = Typography;

/// 新建模板预填的最小合法 WorkflowSpec（id 固定 wf-example，todo t1）。
export const EXAMPLE_SPEC = {
  schema_version: 1,
  id: 'wf-example',
  name: '示例工作流',
  objective: '示例目标：完成一件事',
  constraints: [],
  todos: [{
    id: 't1',
    title: '示例任务',
    requirement_background: '',
    instructions: '描述这个任务要做什么',
    depends_on: [],
    agent: 'act',
    max_attempts: 3,
    acceptance: { criteria: '完成即通过' },
  }],
  metadata: {},
};

function CreateTemplateForm({ onNotice, onCreated }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  const submit = async (values) => {
    let spec = null;
    try {
      spec = JSON.parse(values.specText || '');
    } catch (e) {
      onNotice('spec JSON 解析失败: ' + (e && e.message));
      return;
    }
    setSaving(true);
    try {
      await apiPost('/api/todo/templates', {
        name: values.name,
        description: values.description || '',
        note: values.note || '',
        spec,
      });
      onCreated();
    } catch (e) {
      onNotice('新建模板失败: ' + (e && e.message)); // 400 = spec 校验失败等
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card size="small" title="新建模板" style={{ marginBottom: 16 }}>
      <Form form={form} layout="vertical" onFinish={submit}
        initialValues={{ specText: JSON.stringify(EXAMPLE_SPEC, null, 2) }}>
        <Row gutter={12}>
          <Col span={6}>
            <Form.Item name="name" label="模板名" rules={[{ required: true, message: '请输入模板名' }]}>
              <Input placeholder="my-template" />
            </Form.Item>
          </Col>
          <Col span={8}>
            <Form.Item name="description" label="描述"><Input /></Form.Item>
          </Col>
          <Col span={8}>
            <Form.Item name="note" label="首版备注"><Input placeholder="v1" /></Form.Item>
          </Col>
        </Row>
        <Form.Item name="specText" label="spec（WorkflowSpec JSON）" rules={[{ required: true, message: '请填写 spec' }]}>
          <TextArea rows={10} style={{ fontFamily: 'monospace' }} aria-label="new-template-spec" />
        </Form.Item>
        <Space>
          <Button type="primary" htmlType="submit" loading={saving}>创建</Button>
          <Text type="secondary">400 时服务端会返回「spec 校验失败: …」</Text>
        </Space>
      </Form>
    </Card>
  );
}

/// 展开行：某模板的版本列表（env 绑定来自 GET /api/todo/templates/:name）。
function VersionsBlock({ template, onNotice, onEdit, onChanged }) {
  const [detail, setDetail] = useState(null);
  const name = template.name;

  useEffect(() => {
    let alive = true;
    apiGet(`/api/todo/templates/${encodeURIComponent(name)}`)
      .then((j) => {
        if (alive) {
          setDetail(j || null);
        }
      })
      .catch((e) => onNotice('获取模板详情失败: ' + (e && e.message)));
    return () => {
      alive = false;
    };
  }, [name, onNotice]);

  const envBy = (detail && detail.env_by_version) || {};
  const versions = (detail && detail.template && detail.template.versions)
    || template.versions
    || [];

  const setCurrent = async (v) => {
    try {
      await apiPut(`/api/todo/templates/${encodeURIComponent(name)}/todo.json`, { current: v });
      onChanged();
    } catch (e) {
      onNotice('设为当前失败: ' + (e && e.message));
    }
  };

  const newVersion = async (sourceVersion) => {
    // 低频操作：备注用 window.prompt 收集，取消即放弃。
    const note = window.prompt('新版本备注', '');
    if (note === null) {
      return;
    }
    try {
      await apiPost(`/api/todo/templates/${encodeURIComponent(name)}/new-version`,
        sourceVersion ? { source_version: sourceVersion, note } : { note });
      onChanged();
    } catch (e) {
      onNotice('新建版本失败: ' + (e && e.message));
    }
  };

  const deleteVersion = async (v) => {
    try {
      await apiDel(`/api/todo/templates/${encodeURIComponent(name)}/${encodeURIComponent(v)}`);
      onChanged();
    } catch (e) {
      // 409 = 删除当前版本
      onNotice(`删除版本 ${v} 失败: ` + (e && e.message));
    }
  };

  const run = async (v) => {
    try {
      const j = await apiPost(`/api/todo/templates/${encodeURIComponent(name)}/${encodeURIComponent(v)}/run`, {});
      onNotice(`已启动工作流: ${(j && j.workflow_id) || ''}`);
      onChanged((j && j.workflow_id) || '');
    } catch (e) {
      onNotice('运行失败: ' + (e && e.message)); // 400 = spec 无效或 env 工具缺失
    }
  };

  return (
    <div style={{ padding: '4px 0' }}>
      {(versions || []).length === 0 ? <Text type="secondary">暂无版本</Text> : versions.map((v) => (
        <div key={v.version} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '2px 0' }}>
          <Tag color={template.current === v.version ? 'blue' : 'default'}>{v.version}</Tag>
          <Text style={{ flex: 1, minWidth: 0 }} ellipsis>{v.note || '(无备注)'}</Text>
          {envBy[v.version] ? <Tag color="purple">env: {envBy[v.version]}</Tag> : <Tag>未绑定 env</Tag>}
          <Space size={0}>
            <Button size="small" type="link" onClick={() => onEdit(name, v.version)}>编辑</Button>
            <Button size="small" type="link" disabled={template.current === v.version} onClick={() => setCurrent(v.version)}>设为当前</Button>
            <Button size="small" type="link" onClick={() => newVersion(v.version)}>新版本</Button>
            <Popconfirm title={`删除版本 ${v.version}？`} onConfirm={() => deleteVersion(v.version)}>
              <Button size="small" type="link" danger>删除版本</Button>
            </Popconfirm>
            <Button size="small" type="link" onClick={() => run(v.version)}>运行</Button>
          </Space>
        </div>
      ))}
      <Button size="small" type="dashed" style={{ marginTop: 6 }} onClick={() => newVersion('')}>+ 从当前新建版本</Button>
    </div>
  );
}

function TemplatesTab({ onNotice, onRan }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState(null); // {name, version} → TodoEditor
  const [creating, setCreating] = useState(false);
  const [bump, setBump] = useState(0);

  const load = useCallback(async (silent) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const j = await apiGet('/api/todo/templates');
      setRows((j && j.templates) || []);
    } catch (e) {
      if (!silent) {
        onNotice('获取模板列表失败: ' + (e && e.message));
      }
    } finally {
      if (!silent) {
        setLoading(false);
      }
    }
  }, [onNotice]);

  useEffect(() => {
    load(false);
  }, [load, bump]);

  const deleteTemplate = async (name) => {
    try {
      await apiDel(`/api/todo/templates/${encodeURIComponent(name)}`);
      setBump((n) => n + 1);
    } catch (e) {
      onNotice('删除模板失败: ' + (e && e.message));
    }
  };

  if (editing) {
    return (
      <TodoEditor
        templateName={editing.name}
        version={editing.version}
        onNotice={onNotice}
        onClose={() => {
          setEditing(null);
          setBump((n) => n + 1);
        }}
      />
    );
  }

  const columns = [
    { title: '名称', dataIndex: 'name', key: 'name' },
    { title: '描述', dataIndex: 'description', key: 'description', ellipsis: true },
    { title: '当前版本', dataIndex: 'current', key: 'current', width: 100,
      render: (v) => <Tag color="blue">{v || '-'}</Tag> },
    { title: '版本数', key: 'versions', width: 80,
      render: (_, r) => <span>{(r.versions || []).length}</span> },
    { title: '操作', key: 'ops', width: 120, render: (_, r) => (
      <Popconfirm title={`删除模板 ${r.name}？`} onConfirm={() => deleteTemplate(r.name)}>
        <Button size="small" danger>删除模板</Button>
      </Popconfirm>
    ) },
  ];

  return (
    <div>
      <Space style={{ marginBottom: 12 }}>
        <Button type="primary" onClick={() => setCreating((v) => !v)}>
          {creating ? '收起新建' : '新建模板'}
        </Button>
      </Space>
      {creating ? (
        <CreateTemplateForm
          onNotice={onNotice}
          onCreated={() => {
            setCreating(false);
            setBump((n) => n + 1);
          }}
        />
      ) : null}
      <Table
        rowKey="name"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        pagination={false}
        expandable={{
          expandedRowRender: (r) => (
            <VersionsBlock
              template={r}
              onNotice={onNotice}
              onEdit={(name, version) => setEditing({ name, version })}
              onChanged={(workflowId) => {
                setBump((n) => n + 1);
                if (workflowId) {
                  onRan(workflowId);
                }
              }}
            />
          ),
        }}
      />
    </div>
  );
}

export function TodoPanel({ onNotice }) {
  const [tab, setTab] = useState('templates');
  const [focusWorkflowId, setFocusWorkflowId] = useState('');

  const onRan = useCallback((workflowId) => {
    setFocusWorkflowId(workflowId || '');
    setTab('runs');
  }, []);

  return (
    <Tabs
      activeKey={tab}
      onChange={setTab}
      items={[
        { key: 'templates', label: '模板', children: <TemplatesTab onNotice={onNotice} onRan={onRan} /> },
        {
          key: 'runs',
          label: '运行',
          children: (
            <TodoRunsPanel
              onNotice={onNotice}
              focusWorkflowId={focusWorkflowId}
              onFocusConsumed={() => setFocusWorkflowId('')}
            />
          ),
        },
      ]}
    />
  );
}
