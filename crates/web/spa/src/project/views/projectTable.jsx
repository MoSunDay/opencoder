import { Button, DatePicker, Input, InputNumber, Select, Space, Table, Tooltip } from 'antd';
import { SearchOutlined } from '@ant-design/icons';
import { useState } from 'react';
import dayjs from 'dayjs';
import { useProjectView } from './viewState.jsx';

function Filter({ column, rows, value, onApply, onClear }) {
  const [draft, setDraft] = useState(value ?? (column.kind === 'enum' ? [] : column.kind === 'text' || !column.kind ? '' : [null, null]));
  const values = column.options || [...new Set(rows.flatMap((row) => {
    const value = column.searchValue(row); return Array.isArray(value) ? value : [value ?? '未关联'];
  }))].map((value) => ({ label: String(value), value }));
  return <div className="project-column-filter" onKeyDown={(event) => event.stopPropagation()}>
    {column.kind === 'enum' ? <Select aria-label={`搜索${column.title}`} mode="multiple" showSearch optionFilterProp="label" value={draft} onChange={setDraft} options={values} style={{ width: '100%' }} />
      : column.kind === 'number' ? <Space><InputNumber aria-label={`${column.title}最小值`} placeholder="最小值" value={draft?.[0]} onChange={(v) => setDraft([v, draft?.[1]])} /><InputNumber aria-label={`${column.title}最大值`} placeholder="最大值" value={draft?.[1]} onChange={(v) => setDraft([draft?.[0], v])} /></Space>
        : column.kind === 'date' ? <DatePicker.RangePicker aria-label={`搜索${column.title}`} value={draft?.map((v) => v ? dayjs(v) : null)} onChange={(v) => setDraft(v?.map((v) => v?.valueOf()) || [null, null])} />
          : <Input aria-label={`搜索${column.title}`} placeholder={`搜索${column.title}`} value={draft} onChange={(event) => setDraft(event.target.value)} onPressEnter={() => onApply(draft)} allowClear />}
    <Space style={{ marginTop: 10 }}><Button size="small" type="primary" onClick={() => onApply(draft)}>搜索</Button><Button size="small" onClick={onClear}>清除</Button></Space>
  </div>;
}
export function matchesColumn(column, row, filter) {
  const value = column.searchValue(row);
  if (column.kind === 'enum') return !filter?.length || (Array.isArray(value) ? value : [value ?? '未关联']).some((v) => filter.includes(v));
  if (column.kind === 'number' || column.kind === 'date') return (filter?.[0] == null || value >= filter[0]) && (filter?.[1] == null || value <= (column.kind === 'date' ? dayjs(filter[1]).endOf('day').valueOf() : filter[1]));
  return String(value ?? '').toLocaleLowerCase().includes(String(filter ?? '').trim().toLocaleLowerCase());
}
export function ProjectTable({ columns, rows, label, onRowClick, pagination, viewKey, ...rest }) {
  const [view, setView] = useProjectView(`table:${viewKey || label}`, { filters: {}, page: 1 });
  const { filters, page } = view;
  const setFilters = (next) => setView((current) => ({ ...current, filters: typeof next === 'function' ? next(current.filters) : next }));
  const setPage = (page) => setView((current) => ({ ...current, page }));
  const shown = rows.filter((row) => columns.every((column) => !Object.prototype.hasOwnProperty.call(filters, column.key) || matchesColumn(column, row, filters[column.key])));
  const active = Object.keys(filters).length > 0;
  const mapped = columns.map(({ kind, searchValue, options, ...column }) => searchValue ? {
    ...column, filteredValue: Object.prototype.hasOwnProperty.call(filters, column.key) ? [filters[column.key]] : null,
    filterIcon: <SearchOutlined aria-label={`筛选${column.title}`} />,
    filterDropdown: ({ close }) => <Filter key={JSON.stringify(filters[column.key])} column={{ ...column, kind, searchValue, options }} rows={rows} value={filters[column.key]}
      onApply={(value) => { setFilters({ ...filters, [column.key]: value }); setPage(1); close(); }}
      onClear={() => { setFilters((current) => { const next = { ...current }; delete next[column.key]; return next; }); setPage(1); close(); }} />,
  } : column);
  return <div className="project-table" aria-label={label}>
    {active && <Button size="small" onClick={() => { setFilters({}); setPage(1); }}>清除全部列筛选</Button>}
    <Table {...rest} rowKey="id" tableLayout="fixed" size="small" columns={mapped} dataSource={shown}
      pagination={pagination === false ? false : { current: page, pageSize: 20, showSizeChanger: false, onChange: setPage, hideOnSinglePage: true }}
      onRow={(row) => ({ onClick: () => onRowClick?.(row), className: onRowClick ? 'project-table-row' : '' })} />
  </div>;
}
export function TableText({ children }) { return <Tooltip title={children}><span className="project-table-text">{children || '未关联'}</span></Tooltip>; }
export const dateColumn = { title: '更新时间', key: 'updated', width: '15%', kind: 'date', searchValue: (row) => row.updated_at, render: (_, row) => <TableText>{dayjs(row.updated_at).format('YYYY-MM-DD HH:mm')}</TableText> };
