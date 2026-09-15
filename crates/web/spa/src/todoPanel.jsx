// todoPanel.jsx — 菜单页「TODO 管理」: 两个 tab。
//   模板 — templates 表 + 展开行版本列表（编辑/设为当前/新版本/删除版本/
//           运行）；新建/编辑都从右侧滑出 100% 宽抽屉（不叠卡片）。
//   运行 — todoRunsPanel.jsx 的工作流列表 + 事件流。
// 版本行上的「运行」成功后带 workflow_id 跳到「运行」tab 并聚焦该工作流。

import { Button, Drawer, Modal, Popconfirm, Space, Table, Tabs, Tag, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPost, apiPut } from './api.js';
import { newId } from './fleet/model.js';
import { TodoEditor } from './todoEditor.jsx';
import { TodoRunsPanel } from './todoRunsPanel.jsx';
import { TodoEnvsPanel } from './envs/todoPanel.jsx';
import { err, info } from './notice.js';
import {FileProblems,errorProblems} from './todo/directory/problems.jsx';

const { Text } = Typography;

export { EXAMPLE_SPEC } from './todo/directory/model.js';

/// 展开行：某模板的版本列表（env 绑定来自 GET /api/todo/templates/:name）。
function VersionsBlock({ template, onNotice, onEdit, onChanged }) {
  const [detail, setDetail] = useState(null);
  const name = template.name;
  const attempts = useRef(new Map());
  const [fileProblems,setFileProblems]=useState([]);

  useEffect(() => {
    let alive = true;
    apiGet(`/api/todo/templates/${encodeURIComponent(name)}`)
      .then((j) => {
        if (alive) {
          setDetail(j || null);
        }
      })
      .catch((e) => onNotice(err('获取模板详情失败: ' + (e && e.message))));
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
      onNotice(err(''));
      onChanged();
    } catch (e) {
      onNotice(err('设为当前失败: ' + (e && e.message)));
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
      onNotice(err(''));
      onChanged();
    } catch (e) {
      onNotice(err('新建版本失败: ' + (e && e.message)));
    }
  };

  const deleteVersion = async (v) => {
    try {
      await apiDel(`/api/todo/templates/${encodeURIComponent(name)}/${encodeURIComponent(v)}`);
      onNotice(err(''));
      onChanged();
    } catch (e) {
      // 409 = 删除当前版本
      onNotice(err(`删除版本 ${v} 失败: ` + (e && e.message)));
    }
  };

  const run = async (v) => {
    if (!attempts.current.has(v)) attempts.current.set(v, newId('todos'));
    try {
      const bundle=await apiGet(`/api/todo/templates/${encodeURIComponent(name)}/${encodeURIComponent(v)}/files`);
      if(bundle.diagnostics?.length){setFileProblems(bundle.diagnostics);return;}
      await apiPost('/api/todo/validate-files',{files:bundle.files});
      const j = await apiPost(`/api/todo/templates/${encodeURIComponent(name)}/${encodeURIComponent(v)}/run`, { id: attempts.current.get(v) });
      attempts.current.delete(v);
      onNotice(info(`已启动工作流: ${(j && j.workflow_id) || ''}`));
      onChanged((j && j.workflow_id) || '');
    } catch (e) {
      setFileProblems(errorProblems(e));
      onNotice(err('运行失败: ' + (e && e.message)));
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
      <FileProblems problems={fileProblems} onClose={()=>setFileProblems([])} onLocate={()=>onEdit(name,template.current)}/>
    </div>
  );
}

function TemplatesTab({ onNotice, onRan }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState(null); // {name, version} → TodoEditor
  const [creating, setCreating] = useState(false);
  const dirty=useRef(false);
  const onDirtyChange=useCallback(value=>{dirty.current=value;},[]);
  const closeDraft=callback=>{
    if(dirty.current)Modal.confirm({title:'放弃未保存的修改？',okText:'放弃修改',cancelText:'继续编辑',onOk:()=>{dirty.current=false;callback();}});
    else callback();
  };
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
        onNotice(err('获取模板列表失败: ' + (e && e.message)));
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
      onNotice(err(''));
      setBump((n) => n + 1);
    } catch (e) {
      onNotice(err('删除模板失败: ' + (e && e.message)));
    }
  };

  const closeEditor = () => closeDraft(()=>{setEditing(null);setBump(n=>n+1);});
  const closeCreate = () => closeDraft(()=>setCreating(false));

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
        <Button type="primary" onClick={() => setCreating(true)}>新建模板</Button>
      </Space>
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
      <Drawer
        title="新建 TODO 模板"
        placement="right"
        open={creating}
        onClose={closeCreate}
        size="100%"
        styles={{ wrapper: { maxWidth: '100vw' } }}
        destroyOnHidden
      >
        <TodoEditor
          creating
          onDirtyChange={onDirtyChange}
          onNotice={onNotice}
          onClose={closeCreate}
          onCreated={() => {
            dirty.current=false;
            setCreating(false);
            setBump((n) => n + 1);
          }}
        />
      </Drawer>
      <Drawer
        title={editing ? `编辑模板 ${editing.name} · ${editing.version}` : ''}
        placement="right"
        open={!!editing}
        onClose={closeEditor}
        size="100%"
        styles={{ wrapper: { maxWidth: '100vw' } }}
        destroyOnHidden
      >
        {editing ? (
          <TodoEditor
            onDirtyChange={onDirtyChange}
            templateName={editing.name}
            version={editing.version}
            onNotice={onNotice}
            onClose={closeEditor}
          />
        ) : null}
      </Drawer>
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
        { key: 'envs', label: '模板环境与工具', children: <TodoEnvsPanel onNotice={onNotice} /> },
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
