// defEditor.jsx — create/edit drawer for a DAG definition: JSON textarea for
// the whole spec, local validation feedback (specValidate.js) plus the
// server's 400 problem list when POST /api/dag/defs rejects the draft.

import { Alert, Button, Drawer, Form, Input, Space, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { parseSpecDraft, problemsFromApiError, validateSpec } from './specValidate.js';

const { Text } = Typography;
const { TextArea } = Input;

const EXAMPLE = `{
  "name": "示例工作流",
  "description": "可选：一段描述",
  "steps": [
    { "name": "fetch", "kind": { "type": "python", "code": "print('hello')" } },
    { "name": "review", "depends_on": ["fetch"], "kind": { "type": "agent", "prompt": "review the artifacts" } }
  ]
}`;

/// Pretty-print a def's spec for the textarea (stable key order via the
/// server's wire shape; extra fields round-trip untouched).
function specToText(def) {
  const spec = def && def.spec ? def.spec : null;
  if (!spec) {
    return EXAMPLE;
  }
  return JSON.stringify(spec, null, 2);
}

/// DefEditor — controlled drawer. `def` null = create; otherwise edit.
/// onSave(spec) must return a Promise: reject (400 problem list) keeps the
/// drawer open with the problems rendered.
export function DefEditor({ open, def, saving, onClose, onSave }) {
  const [text, setText] = useState(EXAMPLE);
  const [problems, setProblems] = useState([]);

  useEffect(() => {
    if (open) {
      setText(specToText(def));
      setProblems([]);
    }
  }, [open, def]);

  const submit = async () => {
    const parsed = parseSpecDraft(text);
    if (parsed.error) {
      setProblems([parsed.error]);
      return;
    }
    const local = validateSpec(parsed.spec);
    if (local.length) {
      setProblems(local);
      return;
    }
    try {
      await onSave(parsed.spec);
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
      size={560}
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
          {'{type: "agent"|"python", ...}'}。步骤名须为小写 slug。
        </Text>
        <Form layout="vertical">
          <Form.Item label="spec (JSON)" validateStatus={problems.length ? 'error' : undefined}>
            <TextArea
              rows={18}
              value={text}
              spellCheck={false}
              onChange={(e) => setText(e.target.value)}
              placeholder="粘贴或编辑工作流 JSON"
              style={{ fontFamily: 'SFMono-Regular, Consolas, monospace', fontSize: 12 }}
            />
          </Form.Item>
        </Form>
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
