// todoEditor.jsx — TODO 模板单版本编辑器：表单 / 画布 / JSON 源码三模式编辑
// GET/PUT /api/todo/templates/:name/:version/context.json 上的 WorkflowSpec。
// 表单模式只覆盖高频字段（name/objective/constraints/todos 常规列）；
// 画布模式（todo/editor/canvasEditor.jsx）可视化编辑节点/依赖，Inspector 内
// 亦可编辑 acceptance.required_tool_calls；JSON 源码模式可编辑完整字段。
// spec state 是唯一草稿事实来源，三模式进出时互相搬运（时序镜像
// dag/defEditor.jsx）；画布坐标是会话状态（positions），不入 spec。
// Env 绑定（env.json）独立保存，成功后回读确认。
// 本组件渲染在 todoPanel 的全宽 Drawer 内，不再自带 Card 外壳：标题由抽屉
// 提供，这里只保留 模式切换 + 返回/保存 工具条。

import { Alert, Button, Divider, Form, Input, Segmented, Select, Space, Spin, Typography } from 'antd';
import { useEffect, useMemo, useState } from 'react';
import { apiGet, apiPut, apiPost } from './api.js';
import { err } from './notice.js';
import { useMessage } from './ui/appMessage.js';
import { MONO_VAR } from './ui/mono.js';
import { TodoCanvasEditor } from './todo/editor/canvasEditor.jsx';
import { validateSpec } from './todo/editor/specValidate.js';

const { TextArea } = Input;
const { Text } = Typography;


import { EXAMPLE_SPEC, specFromContext, specToForm, formToSpec } from './todo/editing/model.js';
export { specFromContext, specToForm, formToSpec } from './todo/editing/model.js';
import { TodoForm } from './todo/editing/form.jsx';

function envOptions(envs) {
  return [{ value: '', label: '不绑定' }].concat(
    (envs || []).map((e) => ({ value: e.name, label: e.name })),
  );
}

function TodoEditorSession({ templateName, version, creating = false, onCreated, onNotice, onClose, onDirtyChange }) {
  /// App 上下文里的 message API（脱离 <App> 时 useMessage 自动回落静态 API）。
  const msg = useMessage();
  const [form] = Form.useForm();
  const [mode, setMode] = useState('canvas');
  const [spec, setSpec] = useState(null);
  const [dirty, setDirty] = useState(false);
  const [newName, setNewName] = useState('');
  const [agentOptions, setAgentOptions] = useState([]);
  const [loadError, setLoadError] = useState('');
  const [envSaving, setEnvSaving] = useState(false);
  const changeSpec = (next) => { setSpec(next); setDirty(true); };
  const [jsonText, setJsonText] = useState('');
  const [positions, setPositions] = useState({}); // 画布会话坐标 map，不入 spec
  const [canvasKey, setCanvasKey] = useState(0); // 进入画布时 bump → 以新 spec 重挂
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [envs, setEnvs] = useState([]);
  const [envBinding, setEnvBinding] = useState('');
  const [envLoaded, setEnvLoaded] = useState('');
  useEffect(() => { onDirtyChange?.(dirty || envBinding !== envLoaded); }, [dirty, envBinding, envLoaded, onDirtyChange]);
  const todosWatch = Form.useWatch('todos', form);

  const ctxPath = `/api/todo/templates/${encodeURIComponent(templateName)}/${encodeURIComponent(version)}/context.json`;
  const envPath = `/api/todo/templates/${encodeURIComponent(templateName)}/${encodeURIComponent(version)}/env.json`;

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const [ctx, envsJ, envJ, agentsJ] = await Promise.all([
          creating ? Promise.resolve(EXAMPLE_SPEC) : apiGet(ctxPath),
          apiGet('/api/todo/envs'),
          creating ? Promise.resolve({env:null}) : apiGet(envPath),
          apiGet('/api/agents'),
        ]);
        if (!alive) {
          return;
        }
        const s = specFromContext(ctx);
        if (!s) {
          throw new Error('context 格式异常（缺少 todos）');
        }
        setSpec(s);
        setAgentOptions((agentsJ.agents || []).filter(a => a.primary && a.name !== 'workflow').map(a => ({value:a.name,label:a.name})));
        form.setFieldsValue(specToForm(s));
        setEnvs((envsJ && envsJ.envs) || []);
        const bound = (envJ && envJ.env) || '';
        setEnvBinding(bound);
        setEnvLoaded(bound);
      } catch (e) {
        setLoadError(e.message);
        if (onNotice) {
          onNotice(err('加载模板版本失败: ' + (e && e.message)));
        }
      } finally {
        if (alive) {
          setLoading(false);
        }
      }
    })();
    return () => {
      alive = false;
    };
  }, [ctxPath, envPath]); // eslint-disable-line react-hooks/exhaustive-deps

  /// depends_on 候选 = 同一 spec 里其它 todo 的 id（todosWatch 实时驱动）。
  const allIds = useMemo(
    () => (Array.isArray(todosWatch) ? todosWatch.map((t) => (t && t.id) || '').filter(Boolean) : []),
    [todosWatch],
  );

  /// 画布红点 / 工具栏徽标的实时校验（结构化 [{path,message}]，来自 specValidate）。
  const problems = useMemo(
    () => (spec && typeof spec === 'object' ? validateSpec(spec) : []),
    [spec],
  );

  /// 三模式切换（时序镜像 dag/defEditor.jsx）：spec 是唯一草稿事实来源，
  /// 离开一个模式先把它的编辑并入 spec，再从（局部变量里的）新 spec 派生
  /// 目标视图 —— setState 异步，绝不读刚 set 过的 state。
  const switchMode = (next) => {
    if (next === mode) {
      return;
    }
    if (mode === 'form') {
      // 离开表单：容忍半填（不 validateFields），当前表单值先并入 spec。
      const merged = formToSpec(form.getFieldsValue(), spec);
      setSpec(merged);
      if (next === 'json') {
        setJsonText(JSON.stringify(merged, null, 2));
      } else {
        setCanvasKey((k) => k + 1); // 重挂画布，让新 spec 成为初始值
      }
      setMode(next);
      return;
    }
    if (mode === 'json') {
      // 离开 JSON：解析失败停留原模式，修好再切。
      let parsed = null;
      try {
        parsed = JSON.parse(jsonText);
      } catch (e) {
        msg.error('JSON 解析失败: ' + (e && e.message));
        return;
      }
      setSpec(parsed);
      if (next === 'form') {
        form.setFieldsValue(specToForm(parsed));
      } else {
        setCanvasKey((k) => k + 1);
      }
      setMode(next);
      return;
    }
    // 离开画布：spec 已是画布持续上抛的草稿，直接派生目标视图。
    if (next === 'form') {
      form.setFieldsValue(specToForm(spec));
    } else {
      setJsonText(JSON.stringify(spec, null, 2));
    }
    setMode(next);
  };

  const save = async () => {
    if (saving || !spec) return;
    let nextSpec = null;
    if (mode === 'form') {
      let values = null;
      try {
        values = await form.validateFields();
      } catch {
        return; // 校验错误已标在字段上
      }
      nextSpec = formToSpec(values, spec);
    } else if (mode === 'canvas') {
      // 画布已把每次结构变更上抛进 spec，直接落盘。
      nextSpec = spec;
    } else {
      try {
        nextSpec = JSON.parse(jsonText);
      } catch (e) {
        msg.error('JSON 解析失败: ' + (e && e.message));
        return;
      }
    }
    const validation = validateSpec(nextSpec);
    if (validation.length) { msg.error(validation.map(p => p.message).join('；')); return; }
    if (creating && !newName.trim()) { msg.error('请输入模板名'); return; }
    setSaving(true);
    try {
      if (creating) {
        await apiPost('/api/todo/templates', {name:newName.trim(),spec:nextSpec,description:'',note:''});
        setDirty(false); onCreated?.(); return;
      }
      await apiPut(ctxPath, nextSpec);
      setSpec(nextSpec);
      if (mode === 'form') form.setFieldsValue(specToForm(nextSpec));
      setDirty(false);
      msg.success('已保存');
    } catch (e) {
      const text = '保存失败: ' + (e && e.message);
      if (onNotice) {
        onNotice(err(text));
      }
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return <div style={{ padding: 24, textAlign: 'center' }}><Spin /></div>;
  }

  return (
    <div className="todo-editor">
      {loadError && <Alert type="error" title="加载模板失败" description={loadError} />}
      {creating && <Form layout="vertical"><Form.Item label="模板名" htmlFor="todo-template-name" required><Input id="todo-template-name" value={newName} onChange={e => {setNewName(e.target.value);setDirty(true);}} placeholder="my-template" /></Form.Item></Form>}
      <div className="todo-parent-heading"><strong>父 Agent · workflow</strong><span>根据目标调度任务，独立验收结果</span></div>
      <div className="todo-editor-toolbar">
        <Segmented
          disabled={saving}
          value={mode}
          onChange={switchMode}
          options={[
            { value: 'form', label: '表单' },
            { value: 'canvas', label: '画布' },
            { value: 'json', label: 'JSON 源码' },
          ]}
        />
        <Space>
          <Button onClick={onClose} disabled={saving}>返回</Button>
          <span className="todo-save-state">{dirty ? '有未保存修改' : '定义已同步'}</span><Button type="primary" loading={saving} disabled={!spec || !!loadError} onClick={save}>{creating ? '创建' : '保存'}</Button>
        </Space>
      </div>
      {mode === 'form' ? (
        <TodoForm form={form} saving={saving} allIds={allIds} todosWatch={todosWatch} agentOptions={agentOptions} onChange={() => setDirty(true)} />
      ) : mode === 'canvas' ? (
        <div className="todo-edit-canvas" inert={saving ? '' : undefined}>
          <TodoCanvasEditor
            key={canvasKey}
            spec={spec}
            problems={problems}
            positions={positions}
            onSpecChange={changeSpec}
            agentOptions={agentOptions}
            onPositionsChange={setPositions}
          />
        </div>
      ) : (
        <div>
          <Text type="secondary">直接编辑 WorkflowSpec JSON；保存前会做本地 JSON 解析检查。</Text>
          <TextArea
            disabled={saving}
            value={jsonText}
            onChange={(e) => {setJsonText(e.target.value);setDirty(true);}}
            rows={24}
            style={{ fontFamily: MONO_VAR, marginTop: 8 }}
            aria-label="spec-json"
          />
        </div>
      )}
      <Divider />
      {!creating && <Space>
        <Text>环境绑定：</Text>
        <Select
          value={envBinding}
          onChange={setEnvBinding}
          options={envOptions(envs)}
          style={{ width: 220 }}
          aria-label="env-binding"
        />
        <Button loading={envSaving} disabled={envBinding === envLoaded} onClick={async () => {
          setEnvSaving(true);
          try { await apiPut(envPath,{env:envBinding || null}); const saved = await apiGet(envPath); setEnvLoaded(saved.env || ''); setEnvBinding(saved.env || ''); msg.success('环境绑定已保存'); }
          catch(e) { msg.error('环境绑定失败：'+e.message); }
          finally { setEnvSaving(false); }
        }}>保存环境绑定</Button>
        <Text type="secondary">{envBinding === envLoaded ? '环境绑定已同步' : '环境绑定未保存'}</Text>
      </Space>}
    </div>
  );
}

export function TodoEditor(props) {
  return <TodoEditorSession key={`${props.templateName}/${props.version}`} {...props} />;
}
