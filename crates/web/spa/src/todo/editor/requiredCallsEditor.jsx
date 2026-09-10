// requiredCallsEditor.jsx — acceptance.required_tool_calls list editor for
// the TODO inspector: the low-frequency field todoEditor.jsx's form mode
// delegates to JSON mode, which this canvas exposes visually. One row =
// tool name Input + arguments_contains JSON TextArea + result_ok Switch +
// delete. The JSON text commits ON BLUR: empty → the key is omitted from
// the emitted call, a valid JSON object → stored as an object, anything
// else → an inline red hint and NO emission (a broken draft never reaches
// the parent). Every commit emits the full immutable calls array.

import { MinusCircleOutlined } from '@ant-design/icons';
import { Button, Input, Space, Switch, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { MONO_VAR } from '../../ui/mono.js';

const { Text } = Typography;
const { TextArea } = Input;

/// argsText(call) → the editable JSON draft for arguments_contains
/// (non-objects and absent values both read as '').
function argsText(call) {
  const v = call && call.arguments_contains;
  return v && typeof v === 'object' && !Array.isArray(v) ? JSON.stringify(v) : '';
}

/// parseArgsText(text) → {omit} | {value} | {error} for the blur commit.
function parseArgsText(text) {
  const raw = String(text == null ? '' : text).trim();
  if (!raw) {
    return { omit: true };
  }
  let v;
  try {
    v = JSON.parse(raw);
  } catch (e) {
    return { error: 'JSON 解析失败: ' + (e && e.message ? e.message : String(e)) };
  }
  if (!v || typeof v !== 'object' || Array.isArray(v)) {
    return { error: 'arguments_contains 必须是 JSON 对象' };
  }
  return { value: v };
}

/// CallRow — one required call. Fully controlled from the parent except
/// the JSON draft (kept local so an invalid text can sit until it is fixed
/// or the committed value moves on, mirroring NameField in stepInspector).
function CallRow({ call, onChange, onRemove }) {
  const incoming = argsText(call);
  const [draft, setDraft] = useState(incoming);
  const [error, setError] = useState('');
  useEffect(() => {
    setDraft(incoming);
    setError('');
  }, [incoming]);
  const commitArgs = () => {
    const parsed = parseArgsText(draft);
    if (parsed.error) {
      setError(parsed.error);
      return; // never emit a broken arguments_contains
    }
    setError('');
    if (parsed.omit) {
      if (call.arguments_contains === undefined) {
        return;
      }
      const next = { ...call };
      delete next.arguments_contains;
      onChange(next);
      return;
    }
    if (JSON.stringify(parsed.value) === JSON.stringify(call.arguments_contains)) {
      return;
    }
    onChange({ ...call, arguments_contains: parsed.value });
  };
  return (
    <div style={{ border: '1px dashed var(--oc-border)', borderRadius: 6, padding: '6px 8px', marginBottom: 6 }}>
      <Input
        size="small"
        placeholder="工具名，如 bash"
        style={{ fontFamily: MONO_VAR }}
        value={(call && call.name) || ''}
        onChange={(e) => onChange({ ...call, name: e.target.value })}
      />
      <TextArea
        size="small"
        rows={2}
        placeholder="{}"
        style={{ fontFamily: MONO_VAR, marginTop: 4 }}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commitArgs}
      />
      {error ? (
        <Text type="danger" style={{ fontSize: 12, display: 'block' }}>
          {error}
        </Text>
      ) : null}
      <Space size={4} style={{ marginTop: 4 }}>
        <Text type="secondary" style={{ fontSize: 12 }}>
          result_ok
        </Text>
        <Switch
          size="small"
          checked={!!call && call.result_ok !== false}
          onChange={(v) => onChange({ ...call, result_ok: v })}
        />
        <Button size="small" type="text" danger icon={<MinusCircleOutlined />} onClick={onRemove}>
          删除
        </Button>
      </Space>
    </div>
  );
}

/// RequiredCallsEditor — controlled list editor for
/// acceptance.required_tool_calls. onChange always receives the complete
/// next array (add/remove/edit included); the parent never sees a draft
/// with an unparseable arguments_contains.
export function RequiredCallsEditor({ calls, onChange }) {
  const list = (Array.isArray(calls) ? calls : []).filter((c) => c && typeof c === 'object');
  return (
    <div>
      {list.map((call, i) => (
        <CallRow
          key={i}
          call={call}
          onChange={(next) => onChange(list.map((c, j) => (j === i ? next : c)))}
          onRemove={() => onChange(list.filter((_, j) => j !== i))}
        />
      ))}
      <Button
        size="small"
        type="dashed"
        block
        onClick={() => onChange(list.concat({ name: '', arguments_contains: {}, result_ok: true }))}
      >
        + 添加工具调用
      </Button>
    </div>
  );
}
