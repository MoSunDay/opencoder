// envsPanel.jsx — 菜单页「Env 管理」：朴素表格列出 env（列检索、头部新建、
// 行内编辑/删除），编辑走抽屉（description / env_vars 键值行；tools 由服务端
// 部分合并语义保留）。工具跟随 env：点行（或工具数 Tag）打开右侧工具抽屉，
// 查看/添加/移除/导入都在该抽屉内完成（见 envs/toolsDrawer.jsx）。
// PUT /api/todo/envs/:name 在工具引用无法解析时 400 —— 服务端 error 经
// onNotice 透出。

import {
  Button, Drawer, Form, Input, Modal, Popconfirm, Space, Table, Tag, Typography,
} from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPost, apiPut } from './api.js';
import { PageShell } from './shell/pageShell.jsx';
import { err } from './notice.js';
import { useEvent } from './ui/editing/useEvent.js';
import { useMessage } from './ui/appMessage.js';
import { tableLoading, tableRows } from './ui/tableLoading.js';
import { envFromContext } from './envs/envModel.js';
import { EnvToolsDrawer } from './envs/toolsDrawer.jsx';

const { TextArea } = Input;
const { Text } = Typography;

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

function CreateEnvModal({ open, onClose, onCreated, onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback);
  const msg = useMessage();
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  const submit = async (values) => {
    setSaving(true);
    try {
      await apiPost('/api/todo/envs', { name: values.name, description: values.description || '' });
      msg.success('已创建');
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

function EnvDrawerSession({ name, open, onClose, onNotice: noticeCallback, onSaved }) {
  const onNotice = useEvent(noticeCallback);
  const msg = useMessage();
  const [description, setDescription] = useState('');
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
      // 只发 description/env_vars：PUT 是部分合并，tools 由服务端保留，
      // 绑定变更一律走工具抽屉（envs/toolsDrawer.jsx）。
      await apiPut(`/api/todo/envs/${encodeURIComponent(name)}`, {
        description,
        env_vars: rowsToVars(rows),
      });
      msg.success('已保存');
      if (onSaved) {
        onSaved();
      }
    } catch (e) {
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
          <Text type="secondary">环境变量（env_vars）</Text>
          <VarRows rows={rows} setRows={setRows} disabled={loading || saving || !loaded} />
        </div>
      </Space>
    </Drawer>
  );
}

export function EnvsPanel({ onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback);
  const msg = useMessage();
  const [envs, setEnvs] = useState([]);
  // env 列表拉取态：只有首屏/显式刷新会遮罩表格 —— 否则拉取中是一片空白。
  // 变更（新建/保存/删除/工具抽屉改动）后的刷新走 silent：遮罩会给表格加
  // pointer-events: none，把行内「编辑」「删除」一起锁死，用户刚点完却点不动
  // 下一行。
  const [loadingEnvs, setLoadingEnvs] = useState(true);
  const [creating, setCreating] = useState(false);
  const [editing, setEditing] = useState('');
  const [toolsEnv, setToolsEnv] = useState('');

  const loadEnvs = useCallback(async (opts) => {
    const silent = !!(opts && opts.silent);
    if (!silent) {
      setLoadingEnvs(true);
    }
    try {
      const j = await apiGet('/api/todo/envs');
      setEnvs((j && j.envs) || []);
    } catch (e) {
      onNotice(err('获取 env 列表失败: ' + (e && e.message)));
    } finally {
      if (!silent) {
        setLoadingEnvs(false);
      }
    }
  }, [onNotice]);

  useEffect(() => {
    loadEnvs();
  }, [loadEnvs]);

  const deleteEnv = async (name) => {
    try {
      await apiDel(`/api/todo/envs/${encodeURIComponent(name)}`);
      msg.success('已删除');
      if (editing === name) {
        setEditing('');
      }
      if (toolsEnv === name) {
        setToolsEnv('');
      }
      loadEnvs({ silent: true });
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
      render: (_, e) => (
        <Tag style={{ cursor: 'pointer' }} onClick={(event) => {
          event.stopPropagation();
          setToolsEnv(e.name);
        }}>{(e.tools || []).length} 个</Tag>
      ) },
    { title: '变量', key: 'vars', width: 90,
      render: (_, e) => <Tag>{Object.keys(e.env_vars || {}).length} 个</Tag> },
    { title: '操作', key: 'ops', width: 140, render: (_, e) => (
      <Space size={0} onClick={(event) => event.stopPropagation()}>
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
        dataSource={tableRows(loadingEnvs, envs)}
        loading={tableLoading(loadingEnvs)}
        pagination={false}
        scroll={{ x: 'max-content' }}
        locale={{ emptyText: '暂无 env' }}
        onRow={(e) => ({ onClick: () => setToolsEnv(e.name), style: { cursor: 'pointer' } })}
      />
      <CreateEnvModal
        open={creating}
        onNotice={onNotice}
        onClose={() => setCreating(false)}
        onCreated={(name) => {
          setCreating(false);
          setEditing(name);
          loadEnvs({ silent: true });
        }}
      />
      <EnvDrawer
        name={editing}
        open={!!editing}
        onNotice={onNotice}
        onClose={() => setEditing('')}
        onSaved={() => loadEnvs({ silent: true })}
      />
      <EnvToolsDrawer
        name={toolsEnv}
        open={!!toolsEnv}
        onNotice={onNotice}
        onClose={() => setToolsEnv('')}
        onChanged={() => loadEnvs({ silent: true })}
      />
    </PageShell>
  );
}

function EnvDrawer(props) {
  return <EnvDrawerSession key={`${props.open}/${props.name}`} {...props} />;
}
