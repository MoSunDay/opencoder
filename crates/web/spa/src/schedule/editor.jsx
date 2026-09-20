// editor.jsx —「调度」页新建/编辑 Modal：cron / kind / target / params(单键
// 文本) / overlap / node_id / enabled。新建隐藏 ID 与时区（id 缺省由后端
// 生成 schedule-<ULID>，时区固定 +08:00）；编辑保留两字段（id 是主键、时区
// 可改）。params 按 kind 分流为普通文本输入：agent/team/todos → prompt
// （team 是话题需求、todos 追加 objective）、dag → args（追加到每个 Wasm 步
// 的命令行）、brain → objective（必填）；字符串值可带 {{now…}} 时间模板。
// 编辑保存按当前键合并 initial.params 的其余键（brain 的 inputs/mode/plan
// 不丢），agent 编辑清掉 how_append 旧键（prompt 经回落机制补进 how.md，
// 避免双源）。结构对齐 project/views/mdModal.jsx：打开重播种、
// validateFields 后把最终值交给 caller 的 onOk——POST-vs-PATCH 的决定权留在
// panel。

import { Button, Form, Input, Modal, Select, Switch } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiPost, apiPut } from '../api.js';
import { KIND_LABELS, nodeOptions } from '../fleet/model.js';
import { err, ok } from '../notice.js';
import { MONO_VAR } from '../ui/mono.js';

const { TextArea } = Input;

/// overlap 序列化为 `skip` | `allow`（crates/core config/schedule.rs）。
export const OVERLAP_LABELS = { skip: '跳过重叠', allow: '允许重叠' };

/// kind 收敛为后端 `ScheduleKind` 的五种变体（project/operator 不被接受，
/// 选中后保存必 400「unknown variant」）。
const KIND_OPTIONS = ['agent', 'team', 'todos', 'dag', 'brain']
  .map((value) => ({ value, label: KIND_LABELS[value] }));
const OVERLAP_OPTIONS = Object.entries(OVERLAP_LABELS).map(([value, label]) => ({ value, label }));
const ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]{0,39}$/;

/// params 按类型单键分流：key 是触发侧的消费键（agent/team/todos 读
/// prompt、dag 读 args、brain 读 objective），label/extra/placeholder 是
/// 各自的表单文案。
const PARAM_FIELDS = {
  agent: {
    key: 'prompt', label: '提示词', required: false,
    extra: '触发时作为首轮消息提交，成功后追加进该 agent 的 how.md',
    placeholder: '每日巡检 {{now-1d:%Y-%m-%d}}',
  },
  team: {
    key: 'prompt', label: '话题需求', required: false,
    extra: '触发时作为 team 话题的首条需求消息',
    placeholder: '本周发布风险盘点',
  },
  todos: {
    key: 'prompt', label: '提示词', required: false,
    extra: '触发时追加到工作流 objective',
    placeholder: '聚焦昨晚的线上告警',
  },
  dag: {
    key: 'args', label: '命令行参数', required: false,
    extra: '触发时追加到每个 Wasm 步的命令行；字符串值可带 {{now…}} 时间模板',
    placeholder: '--date {{now-1d:%Y-%m-%d}}',
  },
  brain: {
    key: 'objective', label: '目标', required: true,
    extra: 'brain 运行的 objective（必填）',
    placeholder: '梳理本周的发布计划',
  },
};
const paramField = (kind) => PARAM_FIELDS[kind] || PARAM_FIELDS.agent;

export function ScheduleEditorModal({ open, initial, nodes, onCancel, onSaved }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const seed = useRef(initial);
  seed.current = initial;
  const recordId = seed.current?.id || 'new';
  const isEdit = seed.current?.id != null;
  const kind = Form.useWatch('kind', form);
  const field = paramField(kind);

  // Re-seed on every open (create ⇒ blank, edit ⇒ record fields). 编辑的
  // params 文本按记录 kind 取消费键（agent 优先展示 how_append ?? prompt，
  // 与触发侧 declared_how_append 的回落顺序一致）。
  useEffect(() => {
    if (!open) return;
    form.resetFields();
    const record = seed.current;
    const seedKind = record?.kind || 'agent';
    const seedParams = record?.params || {};
    const seedText = seedKind === 'agent'
      ? (typeof seedParams.how_append === 'string' && seedParams.how_append) || (typeof seedParams.prompt === 'string' && seedParams.prompt) || ''
      : (typeof seedParams[paramField(seedKind).key] === 'string' && seedParams[paramField(seedKind).key]) || '';
    form.setFieldsValue({
      id: record?.id || '',
      cron: record?.cron || '',
      timezone: record?.timezone || '',
      kind: seedKind,
      target: record?.target || '',
      params: isEdit ? seedText : '',
      overlap: record?.overlap || 'skip',
      node_id: record?.node_id || '',
      enabled: record?.enabled ?? true,
    });
    setSaving(false);
  }, [open, recordId, form]);

  // params 映射：新建只写当前键；编辑合并 initial.params 的其余键（清空
  // 文本则删该键），agent 顺带清掉 how_append 旧键（单一来源是 prompt）。
  const paramsOf = (values) => {
    const key = paramField(values.kind).key;
    const text = (values.params || '').trim();
    if (!isEdit) return text ? { [key]: text } : {};
    const merged = { ...(seed.current?.params || {}) };
    if (text) merged[key] = text;
    else delete merged[key];
    if (values.kind === 'agent') delete merged.how_append;
    return merged;
  };

  const save = async () => {
    if (saving) return;
    let values;
    try { values = await form.validateFields(); }
    catch { return; } // antd 已在字段下方钉出必填/格式错误
    setSaving(true);
    try {
      const body = {
        // 新建省略 id（后端生成 schedule-<ULID>）且时区固定 +08:00；编辑
        // 保持现语义（id 走 PUT 路径主键、时区可改）。
        id: isEdit ? (values.id?.trim() || undefined) : undefined,
        cron: values.cron.trim(),
        timezone: isEdit ? (values.timezone?.trim() || null) : '+08:00',
        kind: values.kind,
        target: values.target.trim(),
        params: paramsOf(values),
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
      {isEdit && <Form.Item
        name="id"
        label="ID"
        rules={[{ pattern: ID_PATTERN, message: '仅限字母数字、-、_，字母数字开头，最长 40 字符' }]}
        extra="ID 即主键，创建后不可改"
      >
        <Input disabled placeholder="nightly-etl" style={{ fontFamily: MONO_VAR }} />
      </Form.Item>}
      <Form.Item
        name="cron"
        label="cron 表达式"
        rules={[{ required: true, message: 'cron 必填' }]}
        extra={`5 段（分 时 日 月 周），可带前导秒段；如 0 3 * * *${isEdit ? '' : '；新建按 UTC+8 解释'}`}
      >
        <Input placeholder="0 3 * * *" style={{ fontFamily: MONO_VAR }} />
      </Form.Item>
      {isEdit && <Form.Item name="timezone" label="时区" extra="固定偏移（UTC、+08:00），留空为 UTC">
        <Input placeholder="+08:00" style={{ fontFamily: MONO_VAR }} />
      </Form.Item>}
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
        label={field.label}
        rules={field.required ? [{ required: true, message: `${field.label}必填` }] : []}
        extra={field.extra}
      >
        <TextArea
          rows={2}
          placeholder={field.placeholder}
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
