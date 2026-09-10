// todoEditor.jsx — TODO 模板单版本编辑器：表单 / 画布 / JSON 源码三模式编辑
// GET/PUT /api/todo/templates/:name/:version/context.json 上的 WorkflowSpec。
// 表单模式只覆盖高频字段（name/objective/constraints/todos 常规列）；
// 画布模式（todo/editor/canvasEditor.jsx）可视化编辑节点/依赖，Inspector 内
// 亦可编辑 acceptance.required_tool_calls；JSON 源码模式兜底全量字段。
// spec state 是唯一草稿事实来源，三模式进出时互相搬运（时序镜像
// dag/defEditor.jsx）；画布坐标是会话状态（positions），不入 spec。
// Env 绑定（env.json）与 context 一起保存（绑定值变化才发 PUT）。

import { Button, Card, Col, Divider, Form, Input, InputNumber, Row, Segmented, Select, Space, Spin, Typography } from 'antd';
import { useEffect, useMemo, useState } from 'react';
import { apiGet, apiPut } from './api.js';
import { err } from './notice.js';
import { useMessage } from './ui/appMessage.js';
import { MONO_VAR } from './ui/mono.js';
import { TodoCanvasEditor } from './todo/editor/canvasEditor.jsx';
import { validateSpec } from './todo/editor/specValidate.js';

const { TextArea } = Input;
const { Text } = Typography;

const AGENT_OPTIONS = ['act', 'plan', 'explore', 'build'].map((a) => ({ value: a, label: a }));
const MAX_ATTEMPTS_MIN = 1;

/// context 响应归一化：裸 spec 或 {spec} 包装都接受，形状不对返回 null。
export function specFromContext(j) {
  if (!j || typeof j !== 'object') {
    return null;
  }
  if (Array.isArray(j.todos)) {
    return j;
  }
  if (j.spec && Array.isArray(j.spec.todos)) {
    return j.spec;
  }
  return null;
}

/// WorkflowSpec → 表单值（todos 展平 acceptance.criteria 到顶层 criteria）。
export function specToForm(s) {
  return {
    name: s.name || '',
    objective: s.objective || '',
    constraints: Array.isArray(s.constraints) ? s.constraints.map(String) : [],
    todos: (Array.isArray(s.todos) ? s.todos : []).map((t) => ({
      id: t.id || '',
      title: t.title || '',
      agent: t.agent || 'act',
      depends_on: Array.isArray(t.depends_on) ? t.depends_on : [],
      max_attempts: Number.isFinite(t.max_attempts) ? t.max_attempts : 3,
      requirement_background: t.requirement_background || '',
      instructions: t.instructions || '',
      criteria: (t.acceptance && t.acceptance.criteria) || '',
    })),
  };
}

/// 表单值 → WorkflowSpec：schema_version/id/metadata 原样保留自 original；
/// required_tool_calls 按 todo id 透传（低频字段，画布/JSON 模式可改）。
export function formToSpec(values, original) {
  const src = original || {};
  const todos = (values.todos || []).map((t) => {
    const prev = (Array.isArray(src.todos) ? src.todos : []).find((p) => p && p.id === t.id);
    const acceptance = { criteria: t.criteria || '' };
    const prevCalls = prev && prev.acceptance && prev.acceptance.required_tool_calls;
    if (Array.isArray(prevCalls)) {
      acceptance.required_tool_calls = prevCalls;
    }
    return {
      id: t.id || '',
      title: t.title || '',
      requirement_background: t.requirement_background || '',
      instructions: t.instructions || '',
      depends_on: Array.isArray(t.depends_on) ? t.depends_on : [],
      agent: t.agent || 'act',
      max_attempts: Number.isFinite(t.max_attempts) ? t.max_attempts : 3,
      acceptance,
    };
  });
  return {
    schema_version: Number.isFinite(src.schema_version) ? src.schema_version : 1,
    id: src.id || '',
    name: values.name || '',
    objective: values.objective || '',
    constraints: Array.isArray(values.constraints) ? values.constraints : [],
    todos,
    metadata: src.metadata && typeof src.metadata === 'object' ? src.metadata : {},
  };
}

function envOptions(envs) {
  return [{ value: '', label: '不绑定' }].concat(
    (envs || []).map((e) => ({ value: e.name, label: e.name })),
  );
}

function TodoEditorSession({ templateName, version, onNotice, onClose }) {
  /// App 上下文里的 message API（脱离 <App> 时 useMessage 自动回落静态 API）。
  const msg = useMessage();
  const [form] = Form.useForm();
  const [mode, setMode] = useState('form');
  const [spec, setSpec] = useState(null);
  const [jsonText, setJsonText] = useState('');
  const [positions, setPositions] = useState({}); // 画布会话坐标 map，不入 spec
  const [canvasKey, setCanvasKey] = useState(0); // 进入画布时 bump → 以新 spec 重挂
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [envs, setEnvs] = useState([]);
  const [envBinding, setEnvBinding] = useState('');
  const [envLoaded, setEnvLoaded] = useState('');
  const todosWatch = Form.useWatch('todos', form);

  const ctxPath = `/api/todo/templates/${encodeURIComponent(templateName)}/${encodeURIComponent(version)}/context.json`;
  const envPath = `/api/todo/templates/${encodeURIComponent(templateName)}/${encodeURIComponent(version)}/env.json`;

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const [ctx, envsJ, envJ] = await Promise.all([
          apiGet(ctxPath),
          apiGet('/api/todo/envs'),
          apiGet(envPath),
        ]);
        if (!alive) {
          return;
        }
        const s = specFromContext(ctx);
        if (!s) {
          throw new Error('context 格式异常（缺少 todos）');
        }
        setSpec(s);
        form.setFieldsValue(specToForm(s));
        setEnvs((envsJ && envsJ.envs) || []);
        const bound = (envJ && envJ.env) || '';
        setEnvBinding(bound);
        setEnvLoaded(bound);
      } catch (e) {
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
    setSaving(true);
    try {
      await apiPut(ctxPath, nextSpec);
      if (envBinding !== envLoaded) {
        // Env 绑定与 context 一起保存；400（env 不存在）走同一错误出口。
        await apiPut(envPath, { env: envBinding || null });
        setEnvLoaded(envBinding);
      }
      setSpec(nextSpec);
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
    return <Card><Spin /></Card>;
  }

  return (
    <Card
      title={`编辑模板 ${templateName} · ${version}`}
      extra={(
        <Space>
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
          <Button onClick={onClose} disabled={saving}>返回</Button>
          <Button type="primary" loading={saving} disabled={!spec} onClick={save}>保存</Button>
        </Space>
      )}
    >
      {mode === 'form' ? (
        <Form form={form} layout="vertical" disabled={saving}>
          <Row gutter={12}>
            <Col span={12}>
              <Form.Item name="name" label="名称" rules={[{ required: true, message: '请输入名称' }]}>
                <Input placeholder="工作流名称" />
              </Form.Item>
            </Col>
          </Row>
          <Form.Item name="objective" label="目标（objective）">
            <TextArea rows={2} placeholder="这个工作流要达成什么" />
          </Form.Item>
          <Form.Item label="约束（constraints）" style={{ marginBottom: 8 }}>
            <Form.List name="constraints">
              {(fields, { add, remove }) => (
                <>
                  {fields.map((f) => (
                    <Space key={f.key} style={{ display: 'flex', marginBottom: 4 }} align="baseline">
                      <Form.Item name={f.name} noStyle>
                        <Input placeholder="约束，如：不得修改 crates/core" style={{ width: 480 }} />
                      </Form.Item>
                      <Button type="link" danger onClick={() => remove(f.name)}>删除</Button>
                    </Space>
                  ))}
                  <Button type="dashed" onClick={() => add('')} style={{ width: 200 }}>+ 添加约束</Button>
                </>
              )}
            </Form.List>
          </Form.Item>
          <Divider orientation="left" plain>TODO 列表</Divider>
          <Form.List name="todos">
            {(fields, { add, remove }) => (
              <>
                {fields.map((f) => {
                  const row = (todosWatch || [])[f.name] || {};
                  const depOptions = allIds.filter((id) => id !== row.id).map((id) => ({ value: id, label: id }));
                  return (
                    <Card key={f.key} size="small" style={{ marginBottom: 12 }}
                      title={`TODO #${f.name + 1}`}
                      extra={<Button type="link" danger onClick={() => remove(f.name)}>删除</Button>}
                    >
                      <Row gutter={12}>
                        <Col span={6}>
                          <Form.Item name={[f.name, 'id']} label="ID" rules={[{ required: true, message: '请输入 ID' }]}>
                            <Input placeholder="t1" />
                          </Form.Item>
                        </Col>
                        <Col span={9}>
                          <Form.Item name={[f.name, 'title']} label="标题" rules={[{ required: true, message: '请输入标题' }]}>
                            <Input />
                          </Form.Item>
                        </Col>
                        <Col span={5}>
                          <Form.Item name={[f.name, 'agent']} label="agent">
                            <Select options={AGENT_OPTIONS} />
                          </Form.Item>
                        </Col>
                        <Col span={4}>
                          <Form.Item name={[f.name, 'max_attempts']} label="最大尝试">
                            <InputNumber min={MAX_ATTEMPTS_MIN} style={{ width: '100%' }} />
                          </Form.Item>
                        </Col>
                        <Col span={24}>
                          <Form.Item name={[f.name, 'depends_on']} label="依赖（depends_on）">
                            <Select mode="multiple" options={depOptions} placeholder="可多选其它 TODO 的 id" />
                          </Form.Item>
                        </Col>
                        <Col span={12}>
                          <Form.Item name={[f.name, 'requirement_background']} label="需求背景">
                            <TextArea rows={3} />
                          </Form.Item>
                        </Col>
                        <Col span={12}>
                          <Form.Item name={[f.name, 'instructions']} label="执行说明">
                            <TextArea rows={3} />
                          </Form.Item>
                        </Col>
                        <Col span={24}>
                          <Form.Item name={[f.name, 'criteria']} label="验收标准（acceptance.criteria）">
                            <TextArea rows={2} />
                          </Form.Item>
                        </Col>
                      </Row>
                    </Card>
                  );
                })}
                <Button type="dashed" onClick={() => add({ id: '', title: '', agent: 'act', depends_on: [], max_attempts: 3, requirement_background: '', instructions: '', criteria: '' })} style={{ width: 200 }}>
                  + 添加 TODO
                </Button>
              </>
            )}
          </Form.List>
          <Divider />
          <Text type="secondary">
            提示：acceptance.required_tool_calls 等低频字段请切换到「画布」（选中节点后在右侧
            Inspector 编辑）或「JSON 源码」模式编辑。
          </Text>
        </Form>
      ) : mode === 'canvas' ? (
        <div className="todo-edit-canvas" inert={saving ? '' : undefined}>
          <TodoCanvasEditor
            key={canvasKey}
            spec={spec}
            problems={problems}
            positions={positions}
            onSpecChange={setSpec}
            onPositionsChange={setPositions}
          />
        </div>
      ) : (
        <div>
          <Text type="secondary">直接编辑 WorkflowSpec JSON；保存前会做本地 JSON 解析检查。</Text>
          <TextArea
            disabled={saving}
            value={jsonText}
            onChange={(e) => setJsonText(e.target.value)}
            rows={24}
            style={{ fontFamily: MONO_VAR, marginTop: 8 }}
            aria-label="spec-json"
          />
        </div>
      )}
      <Divider />
      <Space>
        <Text>Env 绑定：</Text>
        <Select
          value={envBinding}
          onChange={setEnvBinding}
          options={envOptions(envs)}
          style={{ width: 220 }}
          aria-label="env-binding"
        />
        <Text type="secondary">随「保存」一并提交</Text>
      </Space>
    </Card>
  );
}

export function TodoEditor(props) {
  return <TodoEditorSession key={`${props.templateName}/${props.version}`} {...props} />;
}
