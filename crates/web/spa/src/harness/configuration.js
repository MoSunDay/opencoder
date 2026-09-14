import { parseEnvs } from './fields.jsx';

// The Web editor manages only the inputs exposed by opencoder --wrap codex.
export function wrapForm(settings = {}) {
  return {
    model: settings.model || '',
    envs: Object.entries(settings.envs || {}).map(([key, value]) => `${key}=${value}`).join('\n'),
  };
}

export function wrapSettings(values) {
  const model = values.model?.trim() || null;
  if (model?.includes('\0')) throw new Error('模型名称不能包含空字符');
  return { model, envs: parseEnvs(values.envs) };
}
