import { Alert, Button, Divider, Drawer, Form, Input, Select, Space, Spin } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet, apiPost, apiPut } from '../api.js';
import { KINDS } from '../fleet/model.js';
import { capabilityBody, capabilityForm, needsTargetSave } from './model.js';

const required = [{ required: true, whitespace: true, message: '请填写此项' }];

function CapabilityEditorSession({ entry, onClose, onSaved }) {
  const [form] = Form.useForm();
  const [id, setId] = useState(entry?.capability?.id || null);
  const [loading, setLoading] = useState(!!id);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [loaded, setLoaded] = useState(!id);
  const [revision, setRevision] = useState(0);
  const originalTarget = useRef(null);
  const initialId = entry?.capability?.id;

  useEffect(() => {
    if (!initialId) return undefined;
    let cancelled = false;
    setLoading(true); setLoaded(false); setError('');
    Promise.all([
      apiGet(`/api/brain/capabilities/${encodeURIComponent(initialId)}`),
      apiGet(`/api/brain/capabilities/${encodeURIComponent(initialId)}/target`),
    ]).then(([detail, binding]) => {
      if (cancelled) return;
      originalTarget.current = binding.target || null;
      form.setFieldsValue(capabilityForm(detail, originalTarget.current));
      setLoaded(true);
    }).catch((e) => { if (!cancelled) setError('读取能力失败: ' + e.message); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [initialId, revision, form]);

  const save = async (values) => {
    if (saving || !loaded) return;
    setSaving(true); setError('');
    let savedId = id;
    let contentSaved = false;
    try {
      const body = capabilityBody(values);
      if (savedId) await apiPut(`/api/brain/capabilities/${encodeURIComponent(savedId)}`, body);
      else {
        const result = await apiPost('/api/brain/capabilities', body);
        savedId = result.capability?.id;
        if (!savedId) throw new Error('服务未返回新能力的 ID');
        setId(savedId);
      }
      contentSaved = true;
      const target = { kind: values.target_kind, target: values.target.trim() };
      if (needsTargetSave(originalTarget.current, target)) {
        await apiPut(`/api/brain/capabilities/${encodeURIComponent(savedId)}/target`, target);
        originalTarget.current = target;
      }
      onSaved();
    } catch (e) {
      setError((contentSaved ? '能力内容已保存，执行目标未保存，请重试: ' : '保存失败: ') + e.message);
    } finally { setSaving(false); }
  };

  return <Drawer open placement="right" size="75%" title={id ? '编辑能力' : '新建能力'}
    styles={{ wrapper: { maxWidth: '100vw' } }} onClose={() => { if (!saving) onClose(); }}
    footer={<Space><Button disabled={saving} onClick={onClose}>取消</Button>
      <Button type="primary" loading={saving} disabled={!loaded} onClick={() => form.submit()}>{id ? '保存修改' : '创建能力'}</Button></Space>}>
    {error && <Alert type="error" showIcon title={error} style={{ marginBottom: 16 }}
      action={!loaded ? <Button onClick={() => setRevision((value) => value + 1)}>重试</Button> : null} />}
    <Spin spinning={loading}>
      <Form form={form} layout="vertical" initialValues={capabilityForm(entry)} onFinish={save} disabled={saving || !loaded}>
        <Form.Item name="capability_type" label="能力类型" rules={required}><Input placeholder="如：代码开发、测试、需求分析" /></Form.Item>
        <Form.Item name="summary" label="一句话描述" rules={required}><Input placeholder="这个能力做什么" /></Form.Item>
        <Form.Item name="input_desc" label="输入描述" rules={required}><Input.TextArea rows={3} placeholder="期望的输入是什么" /></Form.Item>
        <Form.Item name="output_desc" label="输出描述" rules={required}><Input.TextArea rows={3} placeholder="产出的结果是什么" /></Form.Item>
        <Form.Item label="工程输入（示例输入）">
          <Form.List name="eng_inputs">{(fields, { add, remove }) => <>
            {fields.map((field) => <div key={field.key} style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
              <Form.Item name={field.name} rules={required} style={{ flex: 1, marginBottom: 0 }}><Input.TextArea autoSize={{ minRows: 1, maxRows: 5 }} placeholder="一条示例输入" /></Form.Item>
              <Button type="text" danger onClick={() => remove(field.name)}>移除</Button>
            </div>)}
            <Button type="dashed" onClick={() => add('')}>添加工程输入</Button>
          </>}</Form.List>
        </Form.Item>
        <Divider titlePlacement="left">执行目标</Divider>
        <Form.Item name="target_kind" label="执行类型" rules={[{ required: true }]}>
          <Select options={KINDS.filter((kind) => ['agent', 'team', 'dag', 'todos'].includes(kind.value))} />
        </Form.Item>
        <Form.Item name="target" label="执行目标" rules={required} extra="默认由 act Agent 执行，也可指定团队或工作流。">
          <Input placeholder="Agent / 团队 / DAG 名称 / 模板名/v1" />
        </Form.Item>
      </Form>
    </Spin>
  </Drawer>;
}

export function CapabilityEditor(props) {
  return <CapabilityEditorSession key={props.entry?.capability?.id || "new"} {...props} />;
}
