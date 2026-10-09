import { Button, Grid, Input, Table, Tag, Typography } from 'antd';
import { ReloadOutlined, SearchOutlined } from '@ant-design/icons';
import { ROLE_LABELS } from './model.js';
import { tableLoading, tableRows } from '../ui/tableLoading.js';
import './styles.css';

export function AdminToolbar({ search, onSearch, placeholder, filters, loading, onRefresh, refreshLabel = '刷新', actions }) {
  return <div className="oc-admin-toolbar">
    <div className="oc-admin-filters">
      <Input className="oc-admin-search" allowClear prefix={<SearchOutlined aria-hidden />} aria-label={placeholder}
        placeholder={placeholder} value={search} onChange={(event) => onSearch(event.target.value)} />
      {filters}
    </div>
    <div className="oc-admin-actions">
      <Button icon={<ReloadOutlined aria-hidden />} loading={loading} onClick={onRefresh}>{refreshLabel}</Button>
      {actions}
    </div>
  </div>;
}
export function AdminTable({ columns, loading, dataSource, ...props }) {
  const screens = Grid.useBreakpoint();
  return <Table size="middle" className="oc-admin-table" tableLayout="fixed"
    loading={typeof loading === 'boolean' ? tableLoading(loading) : loading}
    dataSource={typeof loading === 'boolean' ? tableRows(loading, dataSource) : dataSource}
    pagination={{ defaultPageSize: 10, showSizeChanger: true, responsive: true, showTotal: (total) => `共 ${total} 条` }}
    columns={columns.map((column) => ({ ...column, fixed: screens.md ? column.fixed : undefined }))}
    {...props} />;
}
export function RoleTag({ role }) {
  return <Tag color={{ admin: 'purple', editor: 'blue', viewer: 'default' }[role]}>{ROLE_LABELS[role] || '—'}</Tag>;
}
export function AdminHint({ children }) {
  return <Typography.Paragraph type="secondary" className="oc-admin-hint">{children}</Typography.Paragraph>;
}
