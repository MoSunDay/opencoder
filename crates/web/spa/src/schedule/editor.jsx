// editor.jsx —「调度」页新建/编辑 Modal：id / cron / timezone / kind /
// target / params(JSON) / overlap / node_id / enabled。结构对齐
// project/views/mdModal.jsx：打开重播种、validateFields 后把最终值交给
// caller 的 onOk——POST-vs-PATCH 的决定权留在 panel。params 用 TextArea +
// JSON.parse 校验（todosTab 的 executor_spec 同款），DAG 目标不收 params
// （后端 validate 拒绝，前端直接禁用该输入）。

import { Button, Form, Input, Modal, Select, Switch } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiPost, apiPut } from '../api.js';
import { CREATABLE_KINDS, KIND_LABELS, nodeOptions } from '../fleet/model.js';
import { err, ok } from '../notice.js';
import { MONO_VAR } from '../ui/mono.js';

const { TextArea } = Input;

/// overlap 序列化为 `skip` | `allow`（crates/core config/schedule.rs）。
export const OVERLAP_LABELS = { skip: '跳过重叠', allow: '允许重叠' };

const KIND_OPTIONS = [...CREATABLE_KINDS, { value: 'brain', label: KIND_LABELS.brain }];
const OVERLAP_OPTIONS = Object.entries(OVERLAP_LABELS).map(([value, label]) => ({ value, label }));
const ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]{0,39}$/;

export function ScheduleEditorModal({ open, initial, nodes, onCancel, onSaved }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const seed = useRef(initial);
  seed.current = initial;
  const recordId = seed.current?.id || 'new';
  const isEdit = seed.current?.id != null;
  const kind = Form.useWatch('kind', form);

  // Re-seed on every open (create ⇒ blank, edit ⇒ record fields).
  useEffect(() => {
    if (!open) return;
    form.resetFields();
    form.setFieldsValue({
      id: seed.current?.id || '',
      cron: seed.current?.cron || '',
      timezone: seed.current?.timezone || '',
      kind: seed.current?.kind || 'agent',
      target: seed.current?.target || '',
      params: seed.current?.params && Object.keys(seed.current.params).length
        ? JSON.stringify(seed.current.params, null, 2) : '',
      overlap: seed.current?.overlap || 'skip',
      node_id: seed.current?.node_id || '',
      enabled: seed.current?.enabled ?? true,
    });
    setSaving(false);
  }, [open, recordId, form]);

  const save = async () => {
    if (saving) return;
    let values;
    try { values = await form.validateFields(); }
    catch { return; } // antd 已在字段下方钉出必填/格式错误
    setSaving(true);
    try {
      const body = {
        id: values.id?.trim() || undefined,
        cron: values.cron.trim(),
        timezone: values.timezone?.trim() || null,
        kind: values.kind,
        target: values.target.trim(),
        // DAG 目标不接收 params（后端 validate 拒绝）；其余目标收 JSON 对象。
        params: values.kind === 'dag' ? {} : parseParams(values.params),
        overlap: values.overlap,
        node_id: values.node_id || null,
        enabled: values.enabled,
      };
      if (isEdit) await apiPut(`/api/schedules/${encodeURIComponent(seed.current.id)}`, body);
      else await apiPost('/api/schedules', body);
      onSaved(ok(isEdit ? '定时任务已保存' : '定时任务已创建'), true);
    }
    catch (e) { onSaved(err('保存定时任务失败: ' + e.message), false); }
    finally { setSaving(false); }
  };

  return <Modal
    open={open}
    title={isEdit ? `编辑定时任务 ${seed.current.id}` : '新建定时任务'}
    width={560}
    onCancel={() => { if (!saving) onCancel(); }}
    destroyOnHidden
    footer={[
      <Button key="cancel" disabled={saving} onClick={onCancel}>取消</Button>,
      <Button key="save" type="primary" loading={saving} onClick={save}>保存</Button>,
    ]}
  >
    <Form form={form} layout="vertical">
      <Form.Item
        name="id"
        label="ID"
        rules={[{ pattern: ID_PATTERN, message: '仅限字母数字、-、_，字母数字开头，最长 40 字符' }]}
        extra={isEdit ? 'ID 即主键，创建后不可改' : '留空自动生成 schedule-<ULID>'}
      >
        <Input disabled={isEdit} placeholder="nightly-etl" style={{ fontFamily: MONO_VAR }} />
      </Form.Item>
      <Form.Item
        name="cron"
        label="cron 表达式"
        rules={[{ required: true, message: 'cron 必填' }]}
        extra="5 段（分 时 日 月 周），可带前导秒段；如 0 3 * * *"
      >
        <Input placeholder="0 3 * * *" style={{ fontFamily: MONO_VAR }} />
      </Form.Item>
      <Form.Item name="timezone" label="时区" extra="固定偏移（UTC、+08:00），留空为 UTC">
        <Input placeholder="+08:00" style={{ fontFamily: MONO_VAR }} />
      </Form.Item>
      <Form.Item name="kind" label="类型" rules={[{ required: true }]}>
        <Select options={KIND_OPTIONS} />
      </Form.Item>
      <Form.Item
        name="target"
        label="目标"
        rules={[{ required: true, message: '目标必填' }]}
        extra="agent/team: 名称；todos: 模板/版本；dag: 定义 id；brain: 计划定义 id"
      >
        <Input aria-label="schedule_target" style={{ fontFamily: MONO_VAR }} />
      </Form.Item>
      <Form.Item
        name="params"
        label="params（JSON 对象）"
        rules={[{
          validator: (_, v) => {
            if (!v || !v.trim()) return Promise.resolve();
            try { JSON.parse(v); return Promise.resolve(); }
            catch (e) { return Promise.reject(new Error(`params 不是合法 JSON: ${e.message}`)); }
          },
        }]}
        extra="字符串值可带 {{now…}} 时间模板；DAG 目标不接收 params"
      >
        <TextArea
          rows={4}
          disabled={kind === 'dag'}
          placeholder='{"prompt": "每日巡检 {{now-1d:%Y-%m-%d}}"}'
          aria-label="schedule_params"
          style={{ fontFamily: MONO_VAR }}
        />
      </Form.Item>
      <Form.Item name="overlap" label="重叠策略">
        <Select options={OVERLAP_OPTIONS} />
      </Form.Item>
      <Form.Item name="node_id" label="指定节点" extra="留空自动调度（活跃 loop / CPU 最低）">
        <Select
          allowClear
          placeholder="自动调度"
          options={nodeOptions(nodes, kind)}
          optionFilterProp="label"
          showSearch
        />
      </Form.Item>
      <Form.Item name="enabled" label="启用" valuePropName="checked">
        <Switch />
      </Form.Item>
    </Form>
  </Modal>;
}

function parseParams(raw) {
  const text = (raw || '').trim();
  if (!text) return {};
  const parsed = JSON.parse(text);
  return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
}
