// notice.test.js — 通知载荷契约单元测试：四个构造器形状固定，
// normalizeNotice 对字符串/对象/非法输入全定义域安全兜底。

import { describe, expect, it } from 'vitest';
import { err, info, normalizeNotice, ok, warn } from './notice.js';

describe('notice constructors', () => {
  it('ok/err/info/warn build the four antd alert types', () => {
    expect(ok('目标已创建')).toEqual({ type: 'success', text: '目标已创建' });
    expect(err('保存失败')).toEqual({ type: 'error', text: '保存失败' });
    expect(info('已开始执行')).toEqual({ type: 'info', text: '已开始执行' });
    expect(warn('请选择所属目标')).toEqual({ type: 'warning', text: '请选择所属目标' });
  });
});

describe('normalizeNotice', () => {
  it('treats bare strings as errors (legacy call sites stay red)', () => {
    expect(normalizeNotice('节点服务不可用')).toEqual({ type: 'error', text: '节点服务不可用' });
    // 清屏惯例 onNotice('')：归一化为空文本错误对象，壳层按无文案不渲染。
    expect(normalizeNotice('')).toEqual({ type: 'error', text: '' });
  });

  it('passes valid notice objects through untouched', () => {
    for (const notice of [ok('目标已创建'), err('失败'), info('已派发'), warn('请选择')]) {
      expect(normalizeNotice(notice)).toBe(notice);
    }
  });

  it('falls back to an empty error notice for illegal inputs', () => {
    expect(normalizeNotice(null)).toEqual({ type: 'error', text: '' });
    expect(normalizeNotice(undefined)).toEqual({ type: 'error', text: '' });
    expect(normalizeNotice({ type: 'success' })).toEqual({ type: 'error', text: '' }); // 缺 text
    expect(normalizeNotice({ type: 'success', text: 42 })).toEqual({ type: 'error', text: '' }); // text 非字符串
    expect(normalizeNotice({ text: '目标已创建' })).toEqual({ type: 'error', text: '' }); // 缺 type
    expect(normalizeNotice({ type: '非法值', text: '目标已创建' })).toEqual({ type: 'error', text: '' }); // type 不在白名单
    expect(normalizeNotice(123)).toEqual({ type: 'error', text: '' });
  });
});
