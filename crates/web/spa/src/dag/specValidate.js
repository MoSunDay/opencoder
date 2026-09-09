// specValidate.js — PURE client-side validation of a DagSpec JSON draft
// (mirror of crates/dag/src/spec.rs rules) before it is POSTed to
// /api/dag/defs. Returns a list of Chinese problem strings; [] means the
// draft may be submitted (the server remains authoritative — its 400
// problem list is surfaced by the editor too).

import { dependsOn, specSteps } from '../dagProjection.js';

export const SLUG_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;

/// Upper bound for agent how_append payloads, in UTF-8 bytes (mirror of
/// crates/dag/src/spec.rs MAX_HOW_APPEND_BYTES).
export const MAX_HOW_APPEND_BYTES = 8 * 1024;

/// parseSpecDraft(text) → {spec} on success or {error} with a readable
/// Chinese message (JSON.parse's own message is English/noisy).
export function parseSpecDraft(text) {
  const raw = String(text || '').trim();
  if (!raw) {
    return { error: '请输入工作流 JSON' };
  }
  let v;
  try {
    v = JSON.parse(raw);
  } catch (e) {
    return { error: 'JSON 解析失败: ' + (e && e.message ? e.message : String(e)) };
  }
  if (!v || typeof v !== 'object' || Array.isArray(v)) {
    return { error: 'spec 必须是 JSON 对象' };
  }
  return { spec: v };
}

/// validateSpec(spec) → problem string list. Checks, in server order:
/// name/description shape, non-empty steps, per-step slug + kind payloads,
/// depends_on references, self-deps and cycles.
export function validateSpec(spec) {
  const problems = [];
  if (!spec || typeof spec !== 'object' || Array.isArray(spec)) {
    return ['spec 必须是 JSON 对象'];
  }
  if (typeof spec.name !== 'string' || !spec.name.trim()) {
    problems.push('spec.name 必须是非空字符串');
  }
  if (spec.description !== undefined && spec.description !== null && typeof spec.description !== 'string') {
    problems.push('spec.description 只能是字符串');
  }
  if (!Array.isArray(spec.steps) || spec.steps.length === 0) {
    problems.push('spec.steps 必须是非空数组');
    return problems;
  }
  const names = new Set();
  spec.steps.forEach((s, i) => {
    const where = 'steps[' + i + ']';
    if (!s || typeof s !== 'object') {
      problems.push(where + ' 必须是对象');
      return;
    }
    if (typeof s.name !== 'string' || !SLUG_RE.test(s.name)) {
      problems.push(where + '.name 必须匹配 [a-z0-9][a-z0-9-]{0,63}: ' + JSON.stringify(s.name));
    } else if (names.has(s.name)) {
      problems.push(where + '.name 重复: ' + s.name);
    }
    names.add(s.name);
    if (!s.kind || typeof s.kind !== 'object') {
      problems.push(where + '.kind 必须是对象');
      return;
    }
    if (s.kind.type === 'agent') {
      if (typeof s.kind.prompt !== 'string' || !s.kind.prompt.trim()) {
        problems.push(where + ' (agent) 需要 non-empty kind.prompt');
      }
      if (s.kind.agent !== undefined && typeof s.kind.agent !== 'string') {
        problems.push(where + '.kind.agent 只能是字符串');
      }
      if (s.kind.model !== undefined && typeof s.kind.model !== 'string') {
        problems.push(where + '.kind.model 只能是字符串');
      }
      if (s.kind.how_append !== undefined) {
        if (typeof s.kind.how_append !== 'string') {
          problems.push(where + '.kind.how_append 只能是字符串');
        } else if (new Blob([s.kind.how_append]).size > MAX_HOW_APPEND_BYTES) {
          problems.push(
            where + ' (agent) kind.how_append 超过 ' + MAX_HOW_APPEND_BYTES + ' 字节上限',
          );
        }
      }
    } else if (s.kind.type === 'runner') {
      for (const key of ['runner', 'agent']) {
        if (typeof s.kind[key] !== 'string' || !SLUG_RE.test(s.kind[key])) {
          problems.push(where + '.kind.' + key + ' 必须是有效的小写 slug');
        }
      }
    } else if (s.kind.type === 'wasm') {
      if (typeof s.kind.command !== 'string' || !s.kind.command.trim()) {
        problems.push(where + ' (wasm) 需要 non-empty kind.command');
      }
      if (s.kind.sandbox !== undefined && !['in_process', 'runc'].includes(s.kind.sandbox)) {
        problems.push(where + '.kind.sandbox 只能是 in_process | runc');
      }
    } else {
      problems.push(where + '.kind.type 必须是 agent | wasm | runner');
    }
    if (s.timeout_secs !== undefined && !(Number.isInteger(s.timeout_secs) && s.timeout_secs > 0)) {
      problems.push(where + '.timeout_secs 必须是正整数');
    }
    if (s.depends_on !== undefined && !Array.isArray(s.depends_on)) {
      problems.push(where + '.depends_on 只能是字符串数组');
    }
  });
  // depends_on references, self-deps, cycles — only meaningful when names parse.
  const known = new Set(specSteps(spec).map((s) => s.name));
  for (const s of specSteps(spec)) {
    dependsOn(s).forEach((d, j) => {
      if (d === s.name) {
        problems.push('steps ' + s.name + ' depends_on 不能包含自身');
      } else if (!known.has(d)) {
        problems.push('steps ' + s.name + ' depends_on 未定义步骤: ' + d);
      }
    });
    if (s.depends_on && Array.isArray(s.depends_on) && new Set(s.depends_on).size !== s.depends_on.length) {
      problems.push('steps ' + s.name + ' depends_on 存在重复项');
    }
  }
  const cycle = findCycle(spec);
  if (cycle) {
    problems.push('依赖存在环: ' + cycle.join(' → '));
  }
  return problems;
}

/// findCycle(spec) → first cycle as a step-name path, or null. DFS with
/// colors; unknown dep names are ignored (reported above instead).
export function findCycle(spec) {
  const steps = specSteps(spec);
  const deps = new Map(steps.map((s) => [s.name, dependsOn(s)]));
  const state = new Map(); // 1 on stack, 2 done
  const path = [];
  const visit = (id) => {
    state.set(id, 1);
    path.push(id);
    for (const d of deps.get(id) || []) {
      if (!deps.has(d)) {
        continue;
      }
      if (!state.has(d)) {
        const found = visit(d);
        if (found) {
          return found;
        }
      } else if (state.get(d) === 1) {
        return [...path.slice(path.indexOf(d)), d];
      }
    }
    path.pop();
    state.set(id, 2);
    return null;
  };
  for (const s of steps) {
    if (!state.has(s.name)) {
      const found = visit(s.name);
      if (found) {
        return found;
      }
    }
  }
  return null;
}

/// problemsFromApiError(e) → string list for a rejected POST /api/dag/defs.
/// The server's 400 carries a problem list; degrade gracefully to whatever
/// error text is available.
export function problemsFromApiError(e) {
  const body = e && e.body;
  if (body && Array.isArray(body.problems) && body.problems.length) {
    return body.problems.map((p) => (typeof p === 'string' ? p : JSON.stringify(p)));
  }
  if (body && typeof body.error === 'string' && body.error) {
    return [body.error];
  }
  if (e && typeof e.message === 'string' && e.message) {
    return [e.message];
  }
  if (typeof e === 'string' && e) {
    return [e];
  }
  return ['提交失败'];
}
