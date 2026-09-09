// stepInspector.jsx — right-hand property panel for the DAG spec editor.
// StepInspector edits ONE selected step (name / kind / prompt / command /
// agent / model / how_append / sandbox / timeout) as fully controlled antd
// inputs inside a vertical Form (no Form instance, no local copy of the
// step); every edit is delegated upward through onChange / onRename /
// onRemove with a fresh immutable step object. SpecMetaForm is the fallback
// panel shown while no node is selected. Problem strings come from
// specValidate.js via the parent.

import { Alert, Button, Form, Input, InputNumber, Popconfirm, Select, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { changeStepKind, renameStep } from './canvasModel.js';

const { Text } = Typography;
const { TextArea } = Input;

const KIND_OPTIONS = [
  { value: 'agent', label: 'Agent 步骤' },
  { value: 'wasm', label: 'Wasm 步骤' },
  { value: 'runner', label: 'Runner 工作流' },
];
const SANDBOX_OPTIONS = [
  { value: 'in_process', label: '内嵌 VM (in_process)' },
  { value: 'runc', label: 'runc 容器' },
];

/// withKindField(step, key, value) → new step with kind[key] set; empty
/// values are REMOVED from the kind payload so the wire shape stays clean.
function withKindField(step, key, value) {
  const kind = { ...(step && step.kind) };
  if (value === undefined || value === null || value === '') {
    delete kind[key];
  } else {
    kind[key] = value;
  }
  return { ...step, kind };
}

/// toTimeout(v) → positive integer for timeout_secs, else undefined.
function toTimeout(v) {
  const n = Math.trunc(Number(v));
  return Number.isFinite(n) && n > 0 ? n : undefined;
}

/// StepInspector — controlled editor for the selected step.
/// onRename(name) commits a valid new name (invalid candidates stay in the
/// input and render the renameStep error inline); onChange(step) commits
/// every other field edit; onRemove() deletes the node.
export function StepInspector({ step, allNames, problemList, onChange, onRename, onRemove }) {
  if (!step || typeof step !== 'object') {
    return null;
  }
  const kind = step.kind || {};
  const kindType = kind.type || '';
  const stepName = typeof step.name === 'string' ? step.name : '';
  const problems = Array.isArray(problemList) ? problemList : [];
  return (
    <div className="dag-edit-inspector">
      {problems.length ? (
        <Alert
          type="error"
          style={{ marginBottom: 10 }}
          message="校验未通过"
          description={
            <ul style={{ margin: 0, paddingLeft: 16 }}>
              {problems.map((p, i) => (
                <li key={i}>{p}</li>
              ))}
            </ul>
          }
        />
      ) : null}
      <NameField stepName={stepName} allNames={allNames} onRename={onRename} />
      <Form layout="vertical" size="small">
        <Form.Item label="类型">
          <Select
            options={KIND_OPTIONS}
            value={kindType}
            onChange={(v) => onChange(changeStepKind(step, v))}
          />
        </Form.Item>
        {kindType === 'agent' ? (
          <>
            <Form.Item label="提示词 (prompt)">
              <TextArea
                rows={3}
                value={kind.prompt || ''}
                onChange={(e) => onChange(withKindField(step, 'prompt', e.target.value))}
              />
            </Form.Item>
            <Form.Item label="Agent 名">
              <Input
                placeholder="可选：自定义 agent 名"
                value={kind.agent || ''}
                onChange={(e) => onChange(withKindField(step, 'agent', e.target.value))}
              />
            </Form.Item>
            <Form.Item label="模型覆盖">
              <Input
                placeholder="可选：模型覆盖"
                value={kind.model || ''}
                onChange={(e) => onChange(withKindField(step, 'model', e.target.value))}
              />
            </Form.Item>
            <Form.Item
              label="经验追加 (how_append)"
              extra={'可选：步骤成功后追加到该 agent 共享池 how.md（≤8KB）'}
            >
              <TextArea
                rows={2}
                placeholder="可选：成功后追加到 how.md 的经验片段"
                value={kind.how_append || ''}
                onChange={(e) => onChange(withKindField(step, 'how_append', e.target.value))}
              />
            </Form.Item>
          </>
        ) : null}
        {kindType === 'runner' && <>
          <Form.Item label="已注册 Runner"><Input value={kind.runner || ''} placeholder="eval-diagnose" onChange={(e) => onChange(withKindField(step, 'runner', e.target.value))} /></Form.Item>
          <Form.Item label="Agent" extra="使用 Agent 绑定的 Codex 配置和资源版本"><Input value={kind.agent || ''} placeholder="eval-diagnose" onChange={(e) => onChange(withKindField(step, 'agent', e.target.value))} /></Form.Item>
        </>}
        {kindType === 'wasm' ? (
          <>
            <Form.Item
              label="Wasm 命令 (command)"
              extra={'格式 <module.wasm> [args...]：模块路径 + 参数'}
            >
              <Input
                placeholder="tool.wasm --flag"
                style={{ fontFamily: 'var(--oc-mono, monospace)' }}
                value={kind.command || ''}
                onChange={(e) => onChange(withKindField(step, 'command', e.target.value))}
              />
            </Form.Item>
            <Form.Item label="沙箱">
              <Select
                allowClear
                placeholder="默认 in_process"
                options={SANDBOX_OPTIONS}
                value={kind.sandbox || undefined}
                onChange={(v) => onChange(withKindField(step, 'sandbox', v))}
              />
            </Form.Item>
          </>
        ) : null}
        <Form.Item label="超时（秒）">
          <InputNumber
            min={1}
            precision={0}
            placeholder="可选（秒）"
            style={{ width: '100%' }}
            value={step.timeout_secs ?? null}
            onChange={(v) => onChange({ ...step, timeout_secs: toTimeout(v) })}
          />
        </Form.Item>
      </Form>
      <Popconfirm
        title="删除该步骤及其依赖连线？"
        okText="删除"
        cancelText="取消"
        onConfirm={onRemove}
      >
        <Button danger block>
          删除步骤
        </Button>
      </Popconfirm>
    </div>
  );
}

/// NameField — the step-name input with inline validation. Valid candidates
/// are committed immediately (onRename); invalid ones stay in the draft and
/// show the renameStep error until fixed or the committed name changes.
function NameField({ stepName, allNames, onRename }) {
  const [draft, setDraft] = useState(stepName);
  const [error, setError] = useState('');
  useEffect(() => {
    setDraft(stepName);
    setError('');
  }, [stepName]);
  const handle = (v) => {
    setDraft(v);
    const err = renameStep(v, allNames);
    setError(err || '');
    if (err === null) {
      onRename(v);
    }
  };
  return (
    <Form layout="vertical" size="small">
      <Form.Item
        label="步骤名"
        validateStatus={error ? 'error' : undefined}
        help={error || undefined}
        style={{ marginBottom: 10 }}
      >
        <Input value={draft} onChange={(e) => handle(e.target.value)} />
      </Form.Item>
    </Form>
  );
}

/// SpecMetaForm — fallback panel while nothing is selected: edits the spec
/// name and optional description through onChange({name} / {description}).
export function SpecMetaForm({ spec, onChange }) {
  const meta = spec && typeof spec === 'object' ? spec : {};
  return (
    <div className="dag-edit-inspector">
      <Text type="secondary" style={{ display: 'block', marginBottom: 8 }}>
        未选中步骤时，可编辑工作流基础信息。
      </Text>
      <Form layout="vertical" size="small">
        <Form.Item label="工作流名称">
          <Input
            value={typeof meta.name === 'string' ? meta.name : ''}
            onChange={(e) => onChange({ name: e.target.value })}
          />
        </Form.Item>
        <Form.Item label="描述（可选）">
          <TextArea
            rows={2}
            value={typeof meta.description === 'string' ? meta.description : ''}
            onChange={(e) => onChange({ description: e.target.value || undefined })}
          />
        </Form.Item>
      </Form>
    </div>
  );
}
