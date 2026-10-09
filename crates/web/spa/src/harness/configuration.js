// Script contents stay outside the platform; only its literal argv is stored.
export function wrapForm(settings = {}) {
  return {
    model: settings.model || '',
    startup_script: (settings.startup_script || []).join('\n'),
  };
}

export function wrapSettings(values) {
  const model = values.model?.trim() || null;
  if (model?.includes('\0')) throw new Error('模型名称不能包含空字符');
  const text = values.startup_script || '';
  const startup_script = text.trim() ? text.replace(/\r\n/g, '\n').split('\n') : [];
  if (startup_script.some((arg) => arg.includes('\0')) || (startup_script.length && !startup_script[0].trim())) {
    throw new Error('启动脚本第一行不能为空，命令和参数不能包含空字符');
  }
  return { model, startup_script };
}
