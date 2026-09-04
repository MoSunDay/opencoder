// envsPanel.jsx — 菜单页「Env 管理」：左侧 env 列表（新建/删除），右侧选中
// env 的编辑器（description / tools 多选（按「已导入」「可导入」分组）/
// env_vars 动态键值行）+ 工具导入区（importable 工具逐条 POST import）。
// PUT /api/todo/envs/:name 在工具引用无法解析时 400 —— 服务端 error 经
// onNotice 透出。

import { Button, Card, Col, Empty, Form, Input, Popconfirm, Row, Select, Space, Table, Tag, Typography, message } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPost, apiPut } from './api.js';

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
export function varsToRows(envVars) {
  return Object.entries(envVars && typeof envVars === 'object' ? envVars : {})
    .map(([k, v]) => [String(k), v === null || v === undefined ? '' : String(v)]);
}

export function rowsToVars(rows) {
  const out = {};
  (rows || []).forEach(([k, v]) => {
    if (k) {
      out[k] = v;
    }
  });
  return out;
}

/// tools 目录 → 多选分组 options（share = 已导入，importable = 可导入）。
export function toolGroupOptions(tools) {
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

function CreateEnvForm({ onNotice, onCreated }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  const submit = async (values) => {
    setSaving(true);
    try {
      await apiPost('/api/todo/envs', { name: values.name, description: values.description || '' });
      onCreated(values.name);
    } catch (e) {
      onNotice('新建 env 失败: ' + (e && e.message));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Form form={form} layout="vertical" onFinish={submit} style={{ marginBottom: 8 }}>
      <Form.Item name="name" label="名称" rules={[{ required: true, message: '请输入名称' }]}>
        <Input placeholder="ffmpeg-env" aria-label="new-env-name" />
      </Form.Item>
      <Form.Item name="description" label="描述">
        <Input placeholder="可选" />
      </Form.Item>
      <Button size="small" type="primary" htmlType="submit" loading={saving}>新建</Button>
    </Form>
  );
}

function VarRows({ rows, setRows }) {
  const update = (i, idx, value) => {
    setRows(rows.map((r, n) => (n === i ? (idx === 0 ? [value, r[1]] : [r[0], value]) : r)));
  };
  const remove = (i) => setRows(rows.filter((_, n) => n !== i));
  const add = () => setRows(rows.concat([['', '']]));
  return (
    <div>
      {rows.map((r, i) => (
        <Space key={i} style={{ display: 'flex', marginBottom: 4 }} align="baseline">
          <Input value={r[0]} placeholder="KEY" style={{ width: 160 }} aria-label="var-key"
            onChange={(e) => update(i, 0, e.target.value)} />
          <Input value={r[1]} placeholder="VALUE" style={{ width: 240 }} aria-label="var-value"
            onChange={(e) => update(i, 1, e.target.value)} />
          <Button type="link" danger aria-label="var-remove" onClick={() => remove(i)}>删除</Button>
        </Space>
      ))}
      <Button type="dashed" onClick={add} style={{ width: 160 }}>+ 添加变量</Button>
    </div>
  );
}

function EnvEditor({ name, tools, onNotice, onSaved }) {
  const [description, setDescription] = useState('');
  const [selectedTools, setSelectedTools] = useState([]);
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    apiGet(`/api/todo/envs/${encodeURIComponent(name)}`)
      .then((j) => {
        if (!alive) {
          return;
        }
        const e = envFromContext(j) || {};
        setDescription(e.description || '');
        setSelectedTools(Array.isArray(e.tools) ? e.tools : []);
        setRows(varsToRows(e.env_vars));
      })
      .catch((e) => onNotice('获取 env 详情失败: ' + (e && e.message)))
      .finally(() => {
        if (alive) {
          setLoading(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [name, onNotice]);

  const save = async () => {
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
      onNotice('保存 env 失败: ' + (e && e.message));
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return <Card size="small"><Text type="secondary">加载中…</Text></Card>;
  }

  return (
    <Card size="small" title={`Env: ${name}`} extra={<Button size="small" type="primary" loading={saving} onClick={save}>保存</Button>}>
      <Space orientation="vertical" style={{ width: '100%' }} size={12}>
        <div>
          <Text type="secondary">描述</Text>
          <TextArea value={description} rows={2} aria-label="env-description"
            onChange={(e) => setDescription(e.target.value)} />
        </div>
        <div>
          <Text type="secondary">工具（tools）</Text>
          <Select mode="multiple" value={selectedTools} options={toolGroupOptions(tools)}
            onChange={setSelectedTools} placeholder="选择已导入工具；可导入项需先导入"
            style={{ width: '100%' }} aria-label="env-tools" />
          <Text type="secondary" style={{ fontSize: 12 }}>
            选择「可导入」组的引用会在保存时被服务端 400 拒绝（工具引用无法解析），请先在下方导入。
          </Text>
        </div>
        <div>
          <Text type="secondary">环境变量（env_vars）</Text>
          <VarRows rows={rows} setRows={setRows} />
        </div>
      </Space>
    </Card>
  );
}

function ToolsSection({ tools, onNotice, onToolsChanged }) {
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
      onNotice('导入工具失败: ' + (e && e.message));
    } finally {
      setImporting('');
    }
  };

  const impCols = [
    { title: 'ref', dataIndex: 'ref', key: 'ref', ellipsis: true,
      render: (v) => <Text style={{ fontFamily: 'monospace', fontSize: 12 }}>{v}</Text> },
    { title: 'agent', dataIndex: 'agent', key: 'agent', width: 120, ellipsis: true },
    { title: 'version', dataIndex: 'version', key: 'version', width: 90, ellipsis: true },
    { title: 'tool', dataIndex: 'tool', key: 'tool', ellipsis: true },
    { title: '操作', key: 'op', width: 80, render: (_, t) => (
      <Button size="small" loading={importing === t.ref} onClick={() => importTool(t)}>导入</Button>
    ) },
  ];

  return (
    <Card size="small" title="工具目录" style={{ marginTop: 12 }}>
      <div style={{ marginBottom: 8 }}>
        <Text type="secondary">已导入（share，只读）：</Text>
        {share.length
          ? share.map((t) => <Tag key={t.ref}>{t.ref}</Tag>)
          : <Text type="secondary">无</Text>}
      </div>
      <Text type="secondary">可导入：</Text>
      <Table rowKey="ref" size="small" columns={impCols} dataSource={importable}
        pagination={false} locale={{ emptyText: '无可导入工具' }} />
    </Card>
  );
}

export function EnvsPanel({ onNotice }) {
  const [envs, setEnvs] = useState([]);
  const [tools, setTools] = useState([]);
  const [selected, setSelected] = useState('');
  const [creating, setCreating] = useState(false);

  const loadEnvs = useCallback(async () => {
    try {
      const j = await apiGet('/api/todo/envs');
      setEnvs((j && j.envs) || []);
    } catch (e) {
      onNotice('获取 env 列表失败: ' + (e && e.message));
    }
  }, [onNotice]);

  const loadTools = useCallback(async () => {
    try {
      const j = await apiGet('/api/todo/tools');
      setTools((j && j.tools) || []);
    } catch (e) {
      onNotice('获取工具目录失败: ' + (e && e.message));
    }
  }, [onNotice]);

  useEffect(() => {
    loadEnvs();
    loadTools();
  }, [loadEnvs, loadTools]);

  const deleteEnv = async (name) => {
    try {
      await apiDel(`/api/todo/envs/${encodeURIComponent(name)}`);
      if (selected === name) {
        setSelected('');
      }
      loadEnvs();
    } catch (e) {
      onNotice('删除 env 失败: ' + (e && e.message));
    }
  };

  return (
    <Row gutter={16}>
      <Col span={8}>
        <Card size="small" title="Env 列表" extra={<Button size="small" onClick={() => setCreating((v) => !v)}>{creating ? '收起' : '新建'}</Button>}>
          {creating ? (
            <CreateEnvForm
              onNotice={onNotice}
              onCreated={(name) => {
                setCreating(false);
                setSelected(name);
                loadEnvs();
              }}
            />
          ) : null}
          {envs.length === 0 ? <Text type="secondary">暂无 env</Text> : envs.map((e) => (
            <div
              key={e.name}
              aria-label={`env-row-${e.name}`}
              onClick={() => setSelected(e.name)}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                padding: '6px 8px',
                cursor: 'pointer',
                background: selected === e.name ? '#e6f4ff' : undefined,
                borderBottom: '1px solid #f5f5f5',
              }}
            >
              <div style={{ flex: 1, minWidth: 0 }}>
                <div><Text strong>{e.name}</Text> <Tag style={{ marginInlineStart: 8 }}>{(e.tools || []).length} 工具</Tag></div>
                <Text type="secondary" style={{ fontSize: 12 }}>{e.description || '-'}</Text>
              </div>
              <Popconfirm title={`删除 env ${e.name}？`} onConfirm={() => deleteEnv(e.name)}>
                <Button size="small" type="link" danger onClick={(ev) => ev.stopPropagation()}>删除</Button>
              </Popconfirm>
            </div>
          ))}
        </Card>
      </Col>
      <Col span={16}>
        {selected
          ? <EnvEditor name={selected} tools={tools} onNotice={onNotice} onSaved={loadEnvs} />
          : <Card size="small"><Empty description="点击左侧 env 进行编辑" /></Card>}
        <ToolsSection tools={tools} onNotice={onNotice} onToolsChanged={loadTools} />
      </Col>
    </Row>
  );
}
