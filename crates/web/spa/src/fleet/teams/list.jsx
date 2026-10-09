import { Button, Input, Space, Table, Tag } from 'antd';
import { useState } from 'react';
import { EditButton } from '../../ui/permissions.jsx';
import { tableLoading, tableRows } from '../../ui/tableLoading.js';

export function TeamList({ rows, loading, onEdit, onLaunch, onRefresh }) {
  const [search, setSearch] = useState('');
  const query = search.trim().toLowerCase();
  const visible = rows.filter((row) => row.name.toLowerCase().includes(query));
  return <>
    <Space wrap style={{ marginBottom: 12, maxWidth: '100%' }}>
      <Input.Search allowClear style={{ width: 220, maxWidth: '100%' }} placeholder="搜索 Team 名称"
        value={search} onChange={(e) => setSearch(e.target.value)} aria-label="team-search" />
      <EditButton type="primary" onClick={() => onEdit(null)}>创建 Team</EditButton>
      <Button onClick={onRefresh}>刷新</Button>
    </Space>
    <Table scroll={{ x: 760 }} rowKey="name" tableLayout="fixed" dataSource={tableRows(loading, visible)} loading={tableLoading(loading)} columns={[
      { title: 'Team', dataIndex: 'name', width: 180, ellipsis: true },
      { title: '成员', width: 260, render: (_, row) => <Space wrap>{(row.members || []).map((m) => <Tag key={m.agent} title={m.agent} style={{ maxWidth: 220, overflow: 'hidden', textOverflow: 'ellipsis' }}>{m.agent}</Tag>)}</Space> },
      { title: '队长', dataIndex: 'captain', width: 160, ellipsis: true },
      { title: '操作', width: 190, render: (_, row) => <Space><EditButton onClick={() => onEdit(row)}>编辑</EditButton><EditButton onClick={() => onLaunch(row)}>启动 Team</EditButton></Space> },
    ]} />
  </>;
}
