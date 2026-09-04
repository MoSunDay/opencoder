// todoEditor.jsx — TODO 模板单版本编辑器：表单 / JSON 源码双模式编辑
// GET/PUT /api/todo/templates/:name/:version/context.json 上的 WorkflowSpec。
// 表单模式只覆盖高频字段（name/objective/constraints/todos 常规列）；
// acceptance.required_tool_calls 属低频字段，不建表单入口 —— 序列化时按
// todo id 从原 spec 透传，需要增删时切到「JSON 源码」模式编辑。
// Env 绑定（env.json）与 context 一起保存（绑定值变化才发 PUT）。

import { Button, Card, Col, Divider, Form, Input, InputNumber, Row, Segmented, Select, Space, Spin, Typography, message } from 'antd';
import { useEffect, useMemo, useState } from 'react';
import { apiGet, apiPut } from './api.js';

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
/// required_tool_calls 按 todo id 透传（低频字段，仅 JSON 模式可改）。
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

export function TodoEditor({ templateName, version, onNotice, onClose }) {
  const [form] = Form.useForm();
  const [mode, setMode] = useState('form');
  const [spec, setSpec] = useState(null);
  const [jsonText, setJsonText] = useState('');
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
          onNotice('加载模板版本失败: ' + (e && e.message));
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

  const switchMode = (next) => {
    if (next === mode) {
      return;
    }
    if (next === 'json') {
      // 表单 → JSON：以当前表单值生成源码（含未保存的编辑）。
      setJsonText(JSON.stringify(formToSpec(form.getFieldsValue(), spec), null, 2));
      setMode('json');
      return;
    }
    let parsed = null;
    try {
      parsed = JSON.parse(jsonText);
    } catch (e) {
      message.error('JSON 解析失败: ' + (e && e.message));
      return; // 停留在 JSON 模式，修好再切
    }
    setSpec(parsed);
    form.setFieldsValue(specToForm(parsed));
    setMode('form');
  };

  const save = async () => {
    let nextSpec = null;
    if (mode === 'form') {
      let values = null;
      try {
        values = await form.validateFields();
      } catch {
        return; // 校验错误已标在字段上
      }
      nextSpec = formToSpec(values, spec);
    } else {
      try {
        nextSpec = JSON.parse(jsonText);
      } catch (e) {
        message.error('JSON 解析失败: ' + (e && e.message));
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
      message.success('已保存');
    } catch (e) {
      const msg = '保存失败: ' + (e && e.message);
      if (onNotice) {
        onNotice(msg);
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
            value={mode}
            onChange={switchMode}
            options={[{ value: 'form', label: '表单' }, { value: 'json', label: 'JSON 源码' }]}
          />
          <Button onClick={onClose}>返回</Button>
          <Button type="primary" loading={saving} onClick={save}>保存</Button>
        </Space>
      )}
    >
      {mode === 'form' ? (
        <Form form={form} layout="vertical">
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
            提示：acceptance.required_tool_calls 等低频字段请切换到「JSON 源码」模式编辑。
          </Text>
        </Form>
      ) : (
        <div>
          <Text type="secondary">直接编辑 WorkflowSpec JSON；保存前会做本地 JSON 解析检查。</Text>
          <TextArea
            value={jsonText}
            onChange={(e) => setJsonText(e.target.value)}
            rows={24}
            style={{ fontFamily: 'monospace', marginTop: 8 }}
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
