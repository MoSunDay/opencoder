// sayText.test.js — 纯逻辑（无 DOM）：Say 头部 preview 的拼接口径 +
// 正文首行去重（渲染修复 ③ 的基础）。DOM 层的断言在
// stepsBlock.dom.test.jsx；这里直接钉 sayPreview/sayBodyParts 的行为。
import { describe, expect, it } from 'vitest';
import { sayBodyParts, sayPreview } from './sayText.js';

const t = (text) => ({ kind: 'text', role: 'assistant', text });
const textsOf = (parts) => parts.filter((p) => p && p.kind === 'text' && !p.image)
  .map((p) => p.text);

describe('sayPreview（头部标签口径）', () => {
  it('拼接全部非 image 文本后取首个非空行并 trim', () => {
    expect(sayPreview([t('  first line  \nsecond line')])).toBe('first line');
    expect(sayPreview([t('\n\nafter blanks')])).toBe('after blanks');
  });

  it('image 标记行不进口径；空/纯空白 Say → 空串', () => {
    expect(sayPreview([{ kind: 'text', role: 'assistant', text: '[image]', image: true }, t('real')]))
      .toBe('real');
    expect(sayPreview([t('   ')])).toBe('');
    expect(sayPreview([])).toBe('');
  });
});

describe('sayBodyParts（正文首行去重，与 preview 同口径）', () => {
  it('多行 Say（首行 == preview）→ 正文只留其余行', () => {
    expect(textsOf(sayBodyParts([t('line one\nline two\nline three')], 'line one')))
      .toEqual(['line two\nline three']);
  });

  it('单行 Say（与 preview 一字不差）→ 正文为空数组', () => {
    expect(sayBodyParts([t('all done here')], 'all done here')).toEqual([]);
    // 尾随换行的单行 Say 也不残留空白块。
    expect(sayBodyParts([t('all done here\n')], 'all done here')).toEqual([]);
    // 首行带前后空白：trim 相等即视为重复。
    expect(sayBodyParts([t('  all done here  ')], 'all done here')).toEqual([]);
  });

  it('首行 != preview → 正文全量渲染', () => {
    expect(textsOf(sayBodyParts([t('line one\nline two')], 'totally different')))
      .toEqual(['line one\nline two']);
    // preview 为空（空 Say）时无从去重，正文全量保留。
    expect(textsOf(sayBodyParts([t('just text')], ''))).toEqual(['just text']);
  });

  it('think/sys/image 部分不属于 preview 口径，原样保留', () => {
    const think = { kind: 'think', role: 'assistant', text: 'plan' };
    const sys = { kind: 'sys', text: 'retried' };
    const img = { kind: 'text', role: 'assistant', text: '[image]', image: true };
    const out = sayBodyParts([think, img, t('head\ntail')], 'head');
    expect(out).toEqual([think, img, t('tail')]);
  });

  it('首行跨部分拼接（image 标记行夹在中间）仍按拼接口径去重', () => {
    const img = { kind: 'text', role: 'assistant', text: '[image]', image: true };
    const out = sayBodyParts([t('head'), img, t('\ntail')], sayPreview([t('head'), t('\ntail')]));
    expect(textsOf(out)).toEqual(['tail']);
  });

  it('不修改输入数组', () => {
    const input = [t('head\ntail')];
    sayBodyParts(input, 'head');
    expect(input).toEqual([t('head\ntail')]);
  });
});
