// tableLoading.test.js — 纯节点单测：表格 loading 约定的两个数值/引用不变量。

import { describe, expect, it } from 'vitest';
import { SPIN_DELAY_MS, tableLoading, tableRows } from './tableLoading.js';

describe('tableLoading convention', () => {
  it('delays the spin long enough to swallow a sub-200ms refresh', () => {
    expect(SPIN_DELAY_MS).toBe(200);
  });

  it('hands antd an object (never a bare boolean) so the delay survives', () => {
    // A bare boolean becomes `delay: 0` inside useSpinProps and flashes.
    expect(tableLoading(false)).toEqual({ spinning: false, delay: SPIN_DELAY_MS });
    expect(tableLoading(true)).toEqual({ spinning: true, delay: SPIN_DELAY_MS });
  });

  it('coerces truthy/falsy fetch state to a boolean spinning flag', () => {
    expect(tableLoading(undefined).spinning).toBe(false);
    expect(tableLoading(0).spinning).toBe(false);
    expect(tableLoading('in-flight').spinning).toBe(true);
    expect(tableLoading({}).spinning).toBe(true);
  });

  it('hides the rows while fetching so the empty state cannot lie', () => {
    // dataSource must be undefined (=== antd EMPTY_LIST path), not [], to
    // suppress 暂无 … during the first paint.
    expect(tableRows(true, [])).toBeUndefined();
    expect(tableRows(true, [{ id: 'a' }])).toBeUndefined();
  });

  it('returns the very same array reference once fetching is done', () => {
    const rows = [{ id: 'a' }, { id: 'b' }];
    expect(tableRows(false, rows)).toBe(rows);
    expect(tableRows(undefined, rows)).toBe(rows);
    expect(tableRows(false, [])).toEqual([]);
  });
});
