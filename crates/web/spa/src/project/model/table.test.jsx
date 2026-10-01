import { expect, it } from 'vitest';
import dayjs from 'dayjs';
import { matchesColumn } from '../views/projectTable.jsx';

it('filters text, multiple tags, inclusive numeric ranges and complete date days', () => {
  const text = { searchValue: (r) => r.title };
  expect(matchesColumn(text, { title: 'Release API' }, ' api ')).toBe(true);
  expect(matchesColumn(text, { title: 'Release API' }, 'web')).toBe(false);
  const tags = { kind: 'enum', searchValue: (r) => r.tags };
  expect(matchesColumn(tags, { tags: ['前端', '重点'] }, ['重点'])).toBe(true);
  expect(matchesColumn(tags, { tags: [] }, ['重点'])).toBe(false);
  const number = { kind: 'number', searchValue: (r) => r.progress };
  expect(matchesColumn(number, { progress: 50 }, [50, 100])).toBe(true);
  expect(matchesColumn(number, { progress: 49 }, [50, null])).toBe(false);
  const date = { kind: 'date', searchValue: (r) => r.updated };
  const day = dayjs('2026-09-30').valueOf();
  expect(matchesColumn(date, { updated: dayjs('2026-09-30T23:59:59').valueOf() }, [day, day])).toBe(true);
  expect(matchesColumn(date, { updated: dayjs('2026-10-01').valueOf() }, [day, day])).toBe(false);
});
