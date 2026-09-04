// reduce.parity.test.js — 纯逻辑（无 DOM），两组契约：
//   (a) 乐观回显去重：chat.jsx 本地插入的 `optimistic` user 回显与随后
//       steer/queue_consumed 帧的同文本回显是同一条边界，折叠去重而不是
//       渲染两条（TUI pending_turn_echo 契约）；pendingEcho 记账不变。
//   (b) live/snapshot 一致性：同一轮对话分别用 reduceFrame 逐帧折叠与
//       turnsFromMessages 快照重建，两条路径的 Say 头部标签序列（含每
//       子轮 steps 计数）必须完全一致 —— 钉死「每子轮计数不累加」。
import { describe, expect, it } from 'vitest';
import { emptyStream, reduceFrame, turnsFromMessages, withUserTurn } from './reduce.js';
import { sayPreview } from './sayText.js';

const userTurns = (turns) => turns.filter((t) => t && t.role === 'user' && t.kind === 'text');

describe('(a) optimistic echo dedup on steer/queue_consumed', () => {
  it('same-text steer_consumed folds into the optimistic echo and clears the flag', () => {
    let s = withUserTurn(emptyStream(), '继续刚才的活', true);
    expect(s.turns[s.turns.length - 1].optimistic).toBe(true);
    s = reduceFrame(s, { event: 'steer_consumed', data: { text: '继续刚才的活' } }, 1);
    // 不产生第二条回显；标记被去掉（该回显从此是权威版本）。
    expect(userTurns(s.turns)).toHaveLength(1);
    expect(userTurns(s.turns)[0]).toMatchObject({ kind: 'text', role: 'user', text: '继续刚才的活' });
    expect('optimistic' in userTurns(s.turns)[0]).toBe(false);
    // pendingEcho 记账语义保持不变。
    expect(s.pendingEcho).toBe('继续刚才的活');
  });

  it('same-text queue_consumed dedups the same way', () => {
    let s = {
      ...emptyStream(),
      turns: [{ kind: 'text', role: 'user', text: '排队任务', optimistic: true }],
    };
    s = reduceFrame(s, { event: 'queue_consumed', data: { text: '排队任务' } }, 1);
    expect(userTurns(s.turns)).toHaveLength(1);
    expect('optimistic' in userTurns(s.turns)[0]).toBe(false);
    expect(s.pendingEcho).toBe('排队任务');
  });

  it('different-text steer_consumed still pushes a fresh echo turn', () => {
    let s = withUserTurn(emptyStream(), '旧的一句', true);
    s = reduceFrame(s, { event: 'steer_consumed', data: { text: '新的一句' } }, 1);
    const echoes = userTurns(s.turns);
    expect(echoes).toHaveLength(2);
    expect(echoes.map((t) => t.text)).toEqual(['旧的一句', '新的一句']);
    // 旧回显不是本帧的权威版本，标记保持原样。
    expect(echoes[0].optimistic).toBe(true);
    expect('optimistic' in echoes[1]).toBe(false);
    expect(s.pendingEcho).toBe('新的一句');
  });

  it('a same-text echo WITHOUT the optimistic flag keeps the plain push (snapshot/steer authority)', () => {
    let s = reduceFrame(emptyStream(), { event: 'steer_consumed', data: { text: '再来一次' } }, 0);
    s = reduceFrame(s, { event: 'steer_consumed', data: { text: '再来一次' } }, 1);
    expect(userTurns(s.turns)).toHaveLength(2);
  });
});

// Say 头部标签：ladder 的 steps 数 + 其收口 Say 的 preview（与
// stepsBlock.jsx 的 L0 标签同一格式；preview 复用 sayText.js 的真实口径）。
function sayLabels(turns) {
  const labels = [];
  for (let i = 0; i < turns.length; i += 1) {
    const turn = turns[i];
    if (!(turn && turn.kind === 'steps' && turn.role === 'assistant')) {
      continue;
    }
    const n = turn.steps.length;
    let say = null;
    for (let j = i + 1; j < turns.length; j += 1) {
      const next = turns[j];
      if (next && next.kind === 'steps') {
        break; // 下一个 ladder 之前没有收口 Say
      }
      if (next && next.kind === 'text' && next.role === 'assistant'
        && !next.image && typeof next.text === 'string' && next.text.trim() !== '') {
        say = next;
        break;
      }
    }
    labels.push(say
      ? `❯ Say(${n} step${n === 1 ? '' : 's'}): ${sayPreview([say])}`
      : `❯ ${n} Step${n === 1 ? '' : 's'}`);
  }
  return labels;
}

describe('(b) live fold vs snapshot replay: per-sub-turn Say counts never accumulate', () => {
  it('yields the identical Say header label sequence on both paths', () => {
    // 直播帧序：三轮 [reasoning → Say → tool]，最后 reasoning → Say 收尾。
    const frames = [
      { event: 'steer_consumed', data: { text: '跑一遍回归' } },
      { event: 'llm_round_start', data: {} },
      { event: 'reasoning_delta', data: { text: '先看目录树' } },
      { event: 'text_delta', data: { text: '第一轮结论' } },
      { event: 'tool_start', data: { id: 'A', name: 'bash', input: { cmd: 'ls' } } },
      { event: 'tool_end', data: { id: 'A', name: 'bash', output: 'src target', is_error: false } },
      { event: 'reasoning_delta', data: { text: '再查一处调用' } },
      { event: 'text_delta', data: { text: '第二轮结论' } },
      { event: 'tool_start', data: { id: 'B', name: 'read', input: { path: 'src/lib.rs' } } },
      { event: 'tool_end', data: { id: 'B', name: 'read', output: 'fn main() {}', is_error: false } },
      { event: 'reasoning_delta', data: { text: '收尾总结' } },
      { event: 'text_delta', data: { text: '第三轮结论' } },
      { event: 'llm_usage', data: { input_tokens: 10, output_tokens: 20 } },
      { event: 'done', data: {} },
    ];
    let s = emptyStream();
    for (const f of frames) {
      s = reduceFrame(s, f, 0);
    }
    expect(s.status).toBe('done');

    // 对应的 store 快照：user text；assistant [reasoning, text, tool_use]×2
    // （tool result 各自成 user 消息）；末轮 assistant [reasoning, text]。
    // 末轮必须带上 reasoning —— 直播帧序里第三轮的 reasoning_delta 会落
    // 进 store 的 assistant 消息，快照少了它两条路径就对不齐。
    const messages = [
      { role: 'user', blocks: [{ kind: 'text', text: '跑一遍回归' }] },
      {
        role: 'assistant',
        blocks: [
          { kind: 'reasoning', text: '先看目录树' },
          { kind: 'text', text: '第一轮结论' },
          { kind: 'tool_use', id: 'A', name: 'bash', input: { cmd: 'ls' } },
        ],
      },
      { role: 'user', blocks: [{ kind: 'tool_result', tool_use_id: 'A', content: 'src target' }] },
      {
        role: 'assistant',
        blocks: [
          { kind: 'reasoning', text: '再查一处调用' },
          { kind: 'text', text: '第二轮结论' },
          { kind: 'tool_use', id: 'B', name: 'read', input: { path: 'src/lib.rs' } },
        ],
      },
      { role: 'user', blocks: [{ kind: 'tool_result', tool_use_id: 'B', content: 'fn main() {}' }] },
      {
        role: 'assistant',
        blocks: [
          { kind: 'reasoning', text: '收尾总结' },
          { kind: 'text', text: '第三轮结论' },
        ],
      },
    ];
    const snap = turnsFromMessages(messages);

    const liveLabels = sayLabels(s.turns);
    const snapLabels = sayLabels(snap);
    // 两条路径的 Say 头部标签序列完全一致（每子轮计数不累加）。
    expect(liveLabels).toEqual(snapLabels);
    expect(liveLabels).toEqual([
      '❯ Say(1 step): 第一轮结论',
      '❯ Say(2 steps): 第二轮结论',
      '❯ Say(2 steps): 第三轮结论',
    ]);
    // steps 计数一致（1 / 2 / 2，而不是跨子轮累加）。
    const counts = (turns) => turns.filter((t) => t && t.kind === 'steps')
      .map((t) => t.steps.length);
    expect(counts(s.turns)).toEqual(counts(snap));
    expect(counts(s.turns)).toEqual([1, 2, 2]);
    // 直播侧：一次 echo 只有一条 user 回显；usage 折叠正常。
    expect(userTurns(s.turns)).toHaveLength(1);
    expect(s.usage).toMatchObject({ input: 10, output: 20 });
  });
});
