export const DEFAULT_TARGET = { kind: 'agent', target: 'act' };

export function capabilityForm(entry, target) {
  const capability = entry?.capability || {};
  return {
    summary: capability.summary || '',
    input_desc: capability.input_desc || '',
    output_desc: capability.output_desc || '',
    eng_inputs: (entry?.eng_inputs || []).map((input) => input.content || ''),
    target_kind: target?.kind || DEFAULT_TARGET.kind,
    target: target?.target || DEFAULT_TARGET.target,
  };
}

export function capabilityBody(values) {
  return {
    // capability_type is derived from the chosen 执行类型 (target_kind);
    // the free-text category field is gone from the editor.
    capability_type: String(values.target_kind || '').trim(),
    summary: String(values.summary || '').trim(),
    input_desc: String(values.input_desc || '').trim(),
    output_desc: String(values.output_desc || '').trim(),
    eng_inputs: (values.eng_inputs || []).map((input) => String(input || '').trim()).filter(Boolean),
  };
}

export function needsTargetSave(original, target) {
  const previous = original || DEFAULT_TARGET;
  return previous.kind !== target.kind || previous.target !== target.target;
}
