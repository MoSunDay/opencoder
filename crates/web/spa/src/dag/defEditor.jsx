// defEditor.jsx — create/edit drawer for a DAG definition with TWO edit
// modes: 画布 (visual canvas — editor/canvasEditor.jsx over a spec draft
// OBJECT, default) and JSON (textarea power mode over the same draft).
// Local validation feedback (specValidate.js) plus the server's 400 problem
// list when POST /api/dag/defs rejects the draft; switching JSON → 画布 is
// blocked while the text does not parse.

import { Alert, Button, Drawer, Form, Input, Segmented, Space, Typography, message } from 'antd';
import { useEffect, useMemo, useState } from 'react';
import { CanvasEditor } from './editor/canvasEditor.jsx';
import { parseSpecDraft, problemsFromApiError, validateSpec } from './specValidate.js';

const { Text } = Typography;
const { TextArea } = Input;

const EXAMPLE = `{
  "name": "示例工作流",
  "description": "可选：一段描述",
  "steps": [
    { "name": "fetch", "kind": { "type": "wasm", "command": "tool.wasm" } },
    { "name": "review", "depends_on": ["fetch"], "kind": { "type": "agent", "prompt": "review the artifacts" } }
  ]
}`;

const EXAMPLE_SPEC = JSON.parse(EXAMPLE);

/// Pretty-print a def's spec for the textarea (stable key order via the
/// server's wire shape; extra fields round-trip untouched).
function specToText(def) {
  const spec = def && def.spec ? def.spec : null;
  if (!spec) {
    return EXAMPLE;
  }
  return JSON.stringify(spec, null, 2);
}

/// DefEditor — controlled drawer, two edit modes: 画布 (visual canvas,
/// default) and JSON (textarea power mode). The canvas maintains a spec
/// draft (object) in this component; the JSON mode edits text that parses
/// back into the same draft. onSave(spec) contract unchanged: reject keeps
/// the drawer open with problems rendered.
export function DefEditor({ open, def, saving, onClose, onSave }) {
  const [mode, setMode] = useState('canvas');
  const [draft, setDraft] = useState(EXAMPLE_SPEC);
  const [text, setText] = useState(EXAMPLE);
  const [problems, setProblems] = useState([]);
  const [positions, setPositions] = useState({});
  const [canvasKey, setCanvasKey] = useState(0);

  // (Re)load on open: def.spec when editing, the shipped example otherwise.
  // canvasKey bump remounts CanvasEditor so the fresh draft is the base.
  useEffect(() => {
    if (!open) {
      return;
    }
    const base = def && def.spec ? def.spec : EXAMPLE_SPEC;
    setDraft(base);
    setText(JSON.stringify(base, null, 2));
    setMode('canvas');
    setProblems([]);
    setPositions({});
    setCanvasKey((k) => k + 1);
  }, [open, def]);

  // Red dots + toolbar badge stay live while the canvas edits the draft.
  const liveProblems = useMemo(
    () => (draft && typeof draft === 'object' && !Array.isArray(draft) ? validateSpec(draft) : []),
    [draft],
  );

  /// 画布 → JSON serializes the draft; JSON → 画布 parses the text back and
  /// REFUSES to switch while it does not parse (problems + warning).
  const switchMode = (m) => {
    if (m === mode) {
      return;
    }
    if (m === 'json') {
      setText(JSON.stringify(draft, null, 2));
      setMode('json');
      return;
    }
    const parsed = parseSpecDraft(text);
    if (parsed.error) {
      setProblems([parsed.error]);
      message.warning('JSON 有误，请先修正后再切换');
      return;
    }
    setDraft(parsed.spec);
    setCanvasKey((k) => k + 1);
    setMode('canvas');
    setProblems([]);
  };

  /// Typing keeps BOTH views coherent: text holds the raw input, draft the
  /// last parse (null while the text is broken).
  const onTextAreaChange = (e) => {
    const v = e.target.value;
    setText(v);
    const parsed = parseSpecDraft(v);
    setDraft(parsed.error ? null : parsed.spec);
  };

  const submit = async () => {
    let specForSave;
    if (mode === 'json') {
      const parsed = parseSpecDraft(text);
      if (parsed.error) {
        setProblems([parsed.error]);
        return;
      }
      specForSave = parsed.spec;
    } else {
      specForSave = draft;
    }
    const local = validateSpec(specForSave);
    if (local.length) {
      setProblems(local);
      return;
    }
    try {
      await onSave(specForSave);
      setProblems([]); // parent closes the drawer on success
    } catch (e) {
      setProblems(problemsFromApiError(e));
    }
  };

  return (
    <Drawer
      title={def ? '编辑工作流定义' : '新建工作流定义'}
      open={open}
      onClose={onClose}
      size={960}
      styles={{ wrapper: { maxWidth: '94vw' } }}
      destroyOnHidden
      footer={
        <Space style={{ float: 'right' }}>
          <Button onClick={onClose}>取消</Button>
          <Button type="primary" loading={saving} onClick={submit}>
            保存
          </Button>
        </Space>
      }
    >
      <Space direction="vertical" size={12} style={{ width: '100%' }}>
        <Text type="secondary">
          spec 为 JSON：name / description? / steps[]，每个 step 为 name、depends_on[]、kind{' '}
          {'{type: "agent"|"wasm", ...}'}。步骤名须为小写 slug。
        </Text>
        <Segmented
          value={mode}
          onChange={(m) => switchMode(m)}
          options={[
            { label: '画布', value: 'canvas' },
            { label: 'JSON', value: 'json' },
          ]}
        />
        {mode === 'canvas' ? (
          <CanvasEditor
            key={canvasKey}
            spec={draft || EXAMPLE_SPEC}
            problems={liveProblems}
            positions={positions}
            onPositionsChange={setPositions}
            onSpecChange={setDraft}
          />
        ) : (
          <Form layout="vertical">
            <Form.Item label="spec (JSON)" validateStatus={problems.length ? 'error' : undefined}>
              <TextArea
                rows={18}
                value={text}
                spellCheck={false}
                onChange={onTextAreaChange}
                placeholder="粘贴或编辑工作流 JSON"
                style={{ fontFamily: 'SFMono-Regular, Consolas, monospace', fontSize: 12 }}
              />
            </Form.Item>
          </Form>
        )}
        {problems.length ? (
          <Alert
            type="error"
            message="spec 校验未通过"
            description={
              <ul style={{ margin: 0, paddingLeft: 18 }}>
                {problems.map((p, i) => (
                  <li key={i}>{p}</li>
                ))}
              </ul>
            }
          />
        ) : null}
      </Space>
    </Drawer>
  );
}
