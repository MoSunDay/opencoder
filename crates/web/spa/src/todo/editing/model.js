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
      _source_id: t.id,
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
  const renamed = new Map((values.todos || []).filter(t => t._source_id).map(t => [t._source_id, t.id]));
  const todos = (values.todos || []).map((t) => {
    const prev = (Array.isArray(src.todos) ? src.todos : []).find((p) => p && p.id === (t._source_id || t.id));
    const acceptance = { ...prev?.acceptance, criteria: t.criteria || '' };
    const prevCalls = prev && prev.acceptance && prev.acceptance.required_tool_calls;
    if (Array.isArray(prevCalls)) {
      acceptance.required_tool_calls = prevCalls;
    }
    return {
      ...prev,
      id: t.id || '',
      title: t.title || '',
      requirement_background: t.requirement_background || '',
      instructions: t.instructions || '',
      depends_on: Array.isArray(t.depends_on) ? t.depends_on.map(id => renamed.get(id) || id) : [],
      agent: t.agent || 'act',
      max_attempts: Number.isFinite(t.max_attempts) ? t.max_attempts : 3,
      acceptance,
    };
  });
  return {
    ...src,
    schema_version: Number.isFinite(src.schema_version) ? src.schema_version : 1,
    id: src.id || '',
    name: values.name || '',
    objective: values.objective || '',
    constraints: Array.isArray(values.constraints) ? values.constraints : [],
    todos,
    metadata: src.metadata && typeof src.metadata === 'object' ? src.metadata : {},
  };
}

export const EXAMPLE_SPEC = {schema_version:1,id:'wf-example',name:'示例工作流',objective:'完成任务并提供可核验结果',constraints:[],
  todos:[{id:'t1',title:'示例任务',requirement_background:'需要完成并验收当前任务',instructions:'完成任务并记录验证结果',depends_on:[],agent:'act',max_attempts:3,acceptance:{criteria:'结果满足目标并有验证依据'},metadata:{}}],metadata:{}};
