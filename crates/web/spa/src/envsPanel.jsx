import { useEvent } from './ui/editing/useEvent.js';
// envsPanel.jsx — 菜单页「Env 管理」：朴素表格列出 env（列检索、头部新建、
// 行内编辑/删除），编辑走抽屉（description / tools 多选 / env_vars 键值行），
// 表下方是工具目录（已导入只读 + 可导入逐条 POST import）。
// PUT /api/todo/envs/:name 在工具引用无法解析时 400 —— 服务端 error 经
// onNotice 透出。

import {
  Button, Drawer, Form, Input, Modal, Popconfirm, Select, Space, Table, Tag, Typography, message,
} from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPost, apiPut } from './api.js';
import { PageShell } from './shell/pageShell.jsx';
import { err } from './notice.js';

const { TextArea } = Input;
const { Text } = Typography;

/// GET /api/todo/envs/:name 的「context object」归一化：{env:{...}} 包装或
/// 裸对象都接受。
export function envFromContext(j) {
  if (!j || typeof j !== 'object') {
    return null;
  }
  const e = j.env && typeof j.env === 'object' ? j.env : j;
  return e && typeof e.name === 'string' ? e : null;
}

/// env_vars 对象 ⇄ 动态行数组 [[k, v], ...]。
function varsToRows(envVars) {
  return Object.entries(envVars && typeof envVars === 'object' ? envVars : {})
    .map(([k, v]) => [String(k), v === null || v === undefined ? '' : String(v)]);
}

function rowsToVars(rows) {
  const out = {};
  (rows || []).forEach(([k, v]) => {
    if (k) {
      out[k] = v;
    }
  });
  return out;
}

/// tools 目录 → 多选分组 options（share = 已导入，importable = 可导入）。
function toolGroupOptions(tools) {
  const share = [];
  const importable = [];
  (tools || []).forEach((t) => {
    if (!t || !t.ref) {
      return;
    }
    if (t.source === 'importable') {
      importable.push({ value: t.ref, label: t.ref });
    } else {
      share.push({ value: t.ref, label: t.ref });
    }
  });
  return [
    { label: '已导入', options: share },
    { label: '可导入', options: importable },
  ];
}

function CreateEnvModal({ open, onClose, onCreated, onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback);
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  const submit = async (values) => {
    setSaving(true);
    try {
      await apiPost('/api/todo/envs', { name: values.name, description: values.description || '' });
      message.success('已创建');
      form.resetFields();
      onCreated(values.name);
    } catch (e) {
      onNotice(err('新建 env 失败: ' + (e && e.message)));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal open={open} title="新建 Env" onCancel={() => { if (!saving) onClose(); }} footer={null} destroyOnHidden>
      <Form form={form} layout="vertical" onFinish={submit} disabled={saving}>
        <Form.Item name="name" label="名称" rules={[{ required: true, message: '请输入名称' }]}>
          <Input placeholder="ffmpeg-env" aria-label="new-env-name" />
        </Form.Item>
        <Form.Item name="description" label="描述">
          <Input placeholder="可选" />
        </Form.Item>
        <Space>
          <Button type="primary" htmlType="submit" loading={saving}>创建</Button>
          <Button onClick={onClose}>取消</Button>
        </Space>
      </Form>
    </Modal>
  );
}

function VarRows({ rows, setRows, disabled }) {
  const update = (i, idx, value) => {
    setRows(rows.map((r, n) => (n === i ? (idx === 0 ? [value, r[1]] : [r[0], value]) : r)));
  };
  const remove = (i) => setRows(rows.filter((_, n) => n !== i));
  const add = () => setRows(rows.concat([['', '']]));
  return (
    <div>
      {rows.map((r, i) => (
        <Space key={i} style={{ display: 'flex', marginBottom: 4 }} align="baseline">
          <Input value={r[0]} placeholder="KEY" style={{ width: 200 }} aria-label="var-key" disabled={disabled}
            onChange={(e) => update(i, 0, e.target.value)} />
          <Input value={r[1]} placeholder="VALUE" style={{ width: 320 }} aria-label="var-value" disabled={disabled}
            onChange={(e) => update(i, 1, e.target.value)} />
          <Button type="link" danger aria-label="var-remove" disabled={disabled} onClick={() => remove(i)}>删除</Button>
        </Space>
      ))}
      <Button type="dashed" onClick={add} disabled={disabled} style={{ width: 200 }}>+ 添加变量</Button>
    </div>
  );
}

function EnvDrawerSession({ name, tools, open, onClose, onNotice: noticeCallback, onSaved }) {
  const onNotice = useEvent(noticeCallback);
  const [description, setDescription] = useState('');
  const [selectedTools, setSelectedTools] = useState([]);
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    if (!open || !name) {
      return;
    }
    let alive = true;
    setLoading(true); setLoaded(false);
    apiGet(`/api/todo/envs/${encodeURIComponent(name)}`)
      .then((j) => {
        if (!alive) {
          return;
        }
        const e = envFromContext(j);
        if (!e) throw new Error('env 详情格式异常');
        setLoaded(true);
        setDescription(e.description || '');
        setSelectedTools(Array.isArray(e.tools) ? e.tools : []);
        setRows(varsToRows(e.env_vars));
      })
      .catch((e) => onNotice(err('获取 env 详情失败: ' + (e && e.message))))
      .finally(() => {
        if (alive) {
          setLoading(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [open, name, onNotice]);

  const save = async () => {
    if (saving || !loaded) return;
    setSaving(true);
    try {
      await apiPut(`/api/todo/envs/${encodeURIComponent(name)}`, {
        description,
        tools: selectedTools,
        env_vars: rowsToVars(rows),
      });
      message.success('已保存');
      if (onSaved) {
        onSaved();
      }
    } catch (e) {
      // 400 = 工具引用无法解析等，服务端 error 字段已并入 e.message
      onNotice(err('保存 env 失败: ' + (e && e.message)));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Drawer
      title={`编辑 Env: ${name || ''}`}
      open={open}
      onClose={() => { if (!saving) onClose(); }}
      width={720}
      destroyOnHidden
      footer={
        <Space style={{ float: 'right' }}>
          <Button onClick={onClose} disabled={saving}>关闭</Button>
          <Button type="primary" loading={saving} disabled={!loaded} onClick={save}>保存</Button>
        </Space>
      }
    >
      <Space orientation="vertical" size={12} style={{ width: '100%' }}>
        <div>
          <Text type="secondary">描述</Text>
          <TextArea value={description} rows={2} aria-label="env-description" disabled={loading || saving}
            onChange={(e) => setDescription(e.target.value)} />
        </div>
        <div>
          <Text type="secondary">工具（tools）</Text>
          <Select mode="multiple" value={selectedTools} options={toolGroupOptions(tools)}
            onChange={setSelectedTools} placeholder="选择已导入工具；可导入项需先导入"
            style={{ width: '100%' }} aria-label="env-tools" disabled={loading || saving} />
          <Text type="secondary" style={{ fontSize: 12 }}>
            选择「可导入」组的引用会在保存时被服务端 400 拒绝（工具引用无法解析），请先在下方导入。
          </Text>
        </div>
        <div>
          <Text type="secondary">环境变量（env_vars）</Text>
          <VarRows rows={rows} setRows={setRows} disabled={loading || saving || !loaded} />
        </div>
      </Space>
    </Drawer>
  );
}

function ToolsCatalog({ tools, onNotice: noticeCallback, onToolsChanged }) {
  const onNotice = useEvent(noticeCallback);
  const [importing, setImporting] = useState('');
  const share = (tools || []).filter((t) => t && t.ref && t.source !== 'importable');
  const importable = (tools || []).filter((t) => t && t.ref && t.source === 'importable');

  const importTool = async (t) => {
    setImporting(t.ref);
    try {
      const j = await apiPost('/api/todo/tools/import', { agent: t.agent, version: t.version, tool: t.tool });
      message.success('已导入: ' + ((j && j.ref) || t.ref));
      if (onToolsChanged) {
        onToolsChanged();
      }
    } catch (e) {
      onNotice(err('导入工具失败: ' + (e && e.message)));
    } finally {
      setImporting('');
    }
  };

  const impCols = [
    { title: 'ref', dataIndex: 'ref', key: 'ref', ellipsis: true,
      filters: importable.map((t) => ({ text: t.ref, value: t.ref })),
      filterSearch: true,
      onFilter: (v, t) => String(t.ref).includes(v),
      render: (v) => <Text style={{ fontFamily: 'monospace', fontSize: 12 }}>{v}</Text> },
    { title: 'agent', dataIndex: 'agent', key: 'agent', width: 120, ellipsis: true },
    { title: 'version', dataIndex: 'version', key: 'version', width: 90, ellipsis: true },
    { title: 'tool', dataIndex: 'tool', key: 'tool', ellipsis: true },
    { title: '操作', key: 'op', width: 80, render: (_, t) => (
      <Button size="small" loading={importing === t.ref} onClick={() => importTool(t)}>导入</Button>
    ) },
  ];

  return (
    <div style={{ marginTop: 16 }}>
      <Space wrap style={{ marginBottom: 8 }}>
        <Text strong>工具目录</Text>
        <Text type="secondary">已导入（share，只读）：</Text>
        {share.length
          ? share.map((t) => <Tag key={t.ref}>{t.ref}</Tag>)
          : <Text type="secondary">无</Text>}
      </Space>
      <Table rowKey="ref" size="small" columns={impCols} dataSource={importable}
        pagination={false} locale={{ emptyText: '无可导入工具' }} />
    </div>
  );
}

export function EnvsPanel({ onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback);
  const [envs, setEnvs] = useState([]);
  const [tools, setTools] = useState([]);
  const [creating, setCreating] = useState(false);
  const [editing, setEditing] = useState('');

  const loadEnvs = useCallback(async () => {
    try {
      const j = await apiGet('/api/todo/envs');
      setEnvs((j && j.envs) || []);
    } catch (e) {
      onNotice(err('获取 env 列表失败: ' + (e && e.message)));
    }
  }, [onNotice]);

  const loadTools = useCallback(async () => {
    try {
      const j = await apiGet('/api/todo/tools');
      setTools((j && j.tools) || []);
    } catch (e) {
      onNotice(err('获取工具目录失败: ' + (e && e.message)));
    }
  }, [onNotice]);

  useEffect(() => {
    loadEnvs();
    loadTools();
  }, [loadEnvs, loadTools]);

  const deleteEnv = async (name) => {
    try {
      await apiDel(`/api/todo/envs/${encodeURIComponent(name)}`);
      message.success('已删除');
      if (editing === name) {
        setEditing('');
      }
      loadEnvs();
    } catch (e) {
      onNotice(err('删除 env 失败: ' + (e && e.message)));
    }
  };

  const columns = [
    { title: '名称', dataIndex: 'name', key: 'name',
      filters: envs.map((e) => ({ text: e.name, value: e.name })),
      filterSearch: true,
      onFilter: (v, e) => String(e.name).includes(v),
      render: (v) => <Text strong>{v}</Text> },
    { title: '描述', dataIndex: 'description', key: 'description', ellipsis: true,
      filters: [...new Set(envs.map((e) => e.description).filter(Boolean))]
        .map((d) => ({ text: d, value: d })),
      filterSearch: true,
      onFilter: (v, e) => String(e.description || '').includes(v),
      render: (v) => v || <Text type="secondary">-</Text> },
    { title: '工具', key: 'tools', width: 90,
      render: (_, e) => <Tag>{(e.tools || []).length} 个</Tag> },
    { title: '变量', key: 'vars', width: 90,
      render: (_, e) => <Tag>{Object.keys(e.env_vars || {}).length} 个</Tag> },
    { title: '操作', key: 'ops', width: 140, render: (_, e) => (
      <Space size={0}>
        <Button size="small" type="link" onClick={() => setEditing(e.name)}>编辑</Button>
        <Popconfirm title={`删除 env ${e.name}？`} okText="确认删除" onConfirm={() => deleteEnv(e.name)}>
          <Button size="small" type="link" danger>删除</Button>
        </Popconfirm>
      </Space>
    ) },
  ];

  return (
    <PageShell
      page="envs"
      extra={<Button type="primary" onClick={() => setCreating(true)}>新建</Button>}
    >
      <Table
        rowKey="name"
        size="small"
        columns={columns}
        dataSource={envs}
        pagination={false}
        scroll={{ x: 'max-content' }}
        locale={{ emptyText: '暂无 env' }}
      />
      <ToolsCatalog tools={tools} onNotice={onNotice} onToolsChanged={loadTools} />
      <CreateEnvModal
        open={creating}
        onNotice={onNotice}
        onClose={() => setCreating(false)}
        onCreated={(name) => {
          setCreating(false);
          setEditing(name);
          loadEnvs();
        }}
      />
      <EnvDrawer
        name={editing}
        tools={tools}
        open={!!editing}
        onNotice={onNotice}
        onClose={() => setEditing('')}
        onSaved={loadEnvs}
      />
    </PageShell>
  );
}

function EnvDrawer(props) {
  return <EnvDrawerSession key={`${props.open}/${props.name}`} {...props} />;
}
