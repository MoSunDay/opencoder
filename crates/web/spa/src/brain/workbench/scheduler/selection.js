import { capabilityId } from './model.js';
import { capabilityLabel } from '../milestone/model.js';

export function capabilityOptions(capabilities, selectionIds, selectedId) {
  const allowed = selectionIds == null ? null : new Set(selectionIds);
  const choices = capabilities.filter((cap) => !allowed || allowed.has(capabilityId(cap)) || capabilityId(cap) === selectedId);
  return choices.map((cap) => {
    const sameTarget = choices.filter((other) => other.kind === cap.kind && other.target === cap.target);
    const contract = (field) => [...new Set(cap[field] || [])].sort().join('、') || '无';
    const label = sameTarget.length > 1
      ? `${capabilityLabel(cap)} · 必填输入：${contract('required_inputs')} · 必填输出：${contract('required_outputs')} · ${capabilityId(cap)}`
      : capabilityLabel(cap);
    return { value: capabilityId(cap), label };
  });
}
