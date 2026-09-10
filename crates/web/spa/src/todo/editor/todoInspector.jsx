// todoInspector.jsx — right-hand property panel for the TODO spec editor.
// TodoInspector edits ONE selected todo (id/title/agent/max_attempts/
// requirement_background/instructions/acceptance.criteria + the
// required_tool_calls editor from requiredCallsEditor.jsx) as fully
// controlled antd inputs inside a vertical Form (no Form instance, no
// local copy of the todo — the id draft and the arguments_contains JSON
// drafts are the only local state); every edit is delegated upward through
// onChange / onRename / onRemove with a fresh immutable todo object.
// SpecMetaForm is the fallback panel shown while no node is selected.
// Problem strings come from specValidate.js via the parent.

import { Alert, Button, Form, Input, InputNumber, Popconfirm, Select, Space, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { RequiredCallsEditor } from './requiredCallsEditor.jsx';
import { BUILTIN_AGENTS } from './specValidate.js';

const { Text } = Typography;
const { TextArea } = Input;

const AGENT_OPTIONS = BUILTIN_AGENTS.map((a) => ({ value: a, label: a }));

/// toAttempts(v) → positive integer for max_attempts, else undefined (an
/// empty input must surface as a validation problem, not silently default).
function toAttempts(v) {
  const n = Math.trunc(Number(v));
  return Number.isFinite(n) && n > 0 ? n : undefined;
}

/// withAcceptance(todo, key, value) → new todo with acceptance[key] set;
/// acceptance is always an object afterwards so nested edits stay simple.
function withAcceptance(todo, key, value) {
  return { ...todo, acceptance: { ...(todo.acceptance || {}), [key]: value } };
}

/// TodoInspector — controlled editor for the selected todo. onRename(id)
/// is committed by TodoIdField on blur/Enter; uniqueness is validated by
/// the PARENT through renameTodo (it sees the whole canvas), failures
/// surface as a warning toast there. allIds is kept for signature parity
/// with the dag StepInspector. onChange(todo) commits every other field
/// edit; onRemove() deletes the node.
export function TodoInspector({ todo, allIds, problemList, onChange, onRename, onRemove }) {
  if (!todo || typeof todo !== 'object') {
    return null;
  }
  const problems = Array.isArray(problemList) ? problemList : [];
  const acceptance = todo.acceptance && typeof todo.acceptance === 'object' ? todo.acceptance : {};
  const calls = Array.isArray(acceptance.required_tool_calls) ? acceptance.required_tool_calls : [];
  return (
    <div className="dag-edit-inspector">
      {problems.length ? (
        <Alert
          type="error"
          style={{ marginBottom: 10 }}
          title="校验未通过"
          description={
            <ul style={{ margin: 0, paddingLeft: 16 }}>
              {problems.map((p, i) => (
                <li key={i}>{p}</li>
              ))}
            </ul>
          }
        />
      ) : null}
      <TodoIdField todoId={todo.id} onRename={onRename} />
      <Form layout="vertical" size="small">
        <Form.Item label="标题 (title)">
          <Input value={todo.title || ''} onChange={(e) => onChange({ ...todo, title: e.target.value })} />
        </Form.Item>
        <Form.Item label="agent">
          <Select options={AGENT_OPTIONS} value={todo.agent || undefined} onChange={(v) => onChange({ ...todo, agent: v })} />
        </Form.Item>
        <Form.Item label="最大尝试 (max_attempts)">
          <InputNumber
            min={1}
            precision={0}
            style={{ width: '100%' }}
            value={todo.max_attempts ?? null}
            onChange={(v) => onChange({ ...todo, max_attempts: toAttempts(v) })}
          />
        </Form.Item>
        <Form.Item label="需求背景 (requirement_background)">
          <TextArea
            rows={3}
            value={todo.requirement_background || ''}
            onChange={(e) => onChange({ ...todo, requirement_background: e.target.value })}
          />
        </Form.Item>
        <Form.Item label="执行说明 (instructions)">
          <TextArea
            rows={3}
            value={todo.instructions || ''}
            onChange={(e) => onChange({ ...todo, instructions: e.target.value })}
          />
        </Form.Item>
        <Form.Item label="验收标准 (acceptance.criteria)">
          <TextArea
            rows={2}
            value={typeof acceptance.criteria === 'string' ? acceptance.criteria : ''}
            onChange={(e) => onChange(withAcceptance(todo, 'criteria', e.target.value))}
          />
        </Form.Item>
        <Form.Item label="required_tool_calls" style={{ marginBottom: 4 }}>
          <RequiredCallsEditor
            calls={calls}
            onChange={(next) => onChange(withAcceptance(todo, 'required_tool_calls', next))}
          />
        </Form.Item>
      </Form>
      <Popconfirm title="删除该 TODO 及其依赖连线？" okText="删除" cancelText="取消" onConfirm={onRemove}>
        <Button danger block>
          删除 TODO
        </Button>
      </Popconfirm>
    </div>
  );
}

/// TodoIdField — the todo id input. Unlike the dag NameField there is no
/// local predicate to run: the rename commits on blur/Enter and simply
/// reverts, because renameTodo (all uniqueness/path rules) lives in the
/// parent where the whole canvas is visible.
function TodoIdField({ todoId, onRename }) {
  const [draft, setDraft] = useState(todoId);
  useEffect(() => {
    setDraft(todoId);
  }, [todoId]);
  const commit = () => {
    const next = draft.trim();
    setDraft(todoId); // revert; a successful rename re-syncs via props
    if (next && next !== todoId) {
      onRename(next);
    }
  };
  return (
    <Form layout="vertical" size="small">
      <Form.Item label="TODO id" extra="失焦或回车提交；重名/非法 id 由画布提示" style={{ marginBottom: 10 }}>
        <Input value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={commit} onPressEnter={commit} />
      </Form.Item>
    </Form>
  );
}

/// SpecMetaForm — fallback panel while nothing is selected: edits the spec
/// name / objective / constraints. Unlike the dag version's per-field
/// partials, onChange always carries the MERGED spec-level fields
/// {name, objective, constraints} — the Form.List rows need the whole
/// array to rebuild anyway, so the parent just folds the object in.
/// Mirrors the dag SpecMetaForm controlled-inputs pattern (no Form
/// instance; the rows are driven by props, Form.List only manages row
/// lifecycle through initialValue).
export function SpecMetaForm({ spec, onChange }) {
  const meta = spec && typeof spec === 'object' ? spec : {};
  const name = typeof meta.name === 'string' ? meta.name : '';
  const objective = typeof meta.objective === 'string' ? meta.objective : '';
  const constraints = Array.isArray(meta.constraints)
    ? meta.constraints.map((c) => (typeof c === 'string' ? c : ''))
    : [];
  const emit = (partial) => onChange({ name, objective, constraints, ...partial });
  return (
    <div className="dag-edit-inspector">
      <Text type="secondary" style={{ display: 'block', marginBottom: 8 }}>
        未选中 TODO 时，可编辑工作流基础信息。
      </Text>
      <Form layout="vertical" size="small">
        <Form.Item label="名称 (name)">
          <Input value={name} onChange={(e) => emit({ name: e.target.value })} />
        </Form.Item>
        <Form.Item label="目标 (objective)">
          <TextArea rows={3} value={objective} onChange={(e) => emit({ objective: e.target.value })} />
        </Form.Item>
        <Form.Item label="约束 (constraints)" style={{ marginBottom: 6 }}>
          <Form.List name="constraints" initialValue={constraints}>
            {(fields, { add, remove }) => (
              <>
                {fields.map((f) => (
                  <Space key={f.key} align="baseline" style={{ display: 'flex', marginBottom: 2 }}>
                    <Input
                      placeholder="约束，如：不得修改 crates/core"
                      value={constraints[f.name] ?? ''}
                      onChange={(e) =>
                        emit({ constraints: constraints.map((s, j) => (j === f.name ? e.target.value : s)) })
                      }
                    />
                    <Button
                      type="link"
                      danger
                      size="small"
                      onClick={() => {
                        remove(f.name);
                        emit({ constraints: constraints.filter((_, j) => j !== f.name) });
                      }}
                    >
                      删除
                    </Button>
                  </Space>
                ))}
                <Button
                  type="dashed"
                  size="small"
                  block
                  onClick={() => {
                    add('');
                    emit({ constraints: constraints.concat('') });
                  }}
                >
                  + 添加约束
                </Button>
              </>
            )}
          </Form.List>
        </Form.Item>
      </Form>
    </div>
  );
}
