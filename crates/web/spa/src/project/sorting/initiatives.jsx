import { useRef, useState } from 'react';
import { Typography } from 'antd';
import { apiPut } from '../../api.js';
import { err } from '../../notice.js';
import { useCanEdit } from '../../ui/permissions.jsx';
import { ProjectTable } from '../views/projectTable.jsx';
import { initiativeColumns } from '../initiativesTab.jsx';
import { SummaryHistory } from '../summary/history.jsx';
import { moveInitiative } from './model.js';

export function ProjectInitiatives({ projectId, rows, refresh, onNotice, openInitiative }) {
  const canEdit = useCanEdit();
  const saving = useRef(false);
  const [preview, setPreview] = useState(null);
  const move = async (activeId, overId) => {
    if (!canEdit || saving.current) return;
    const next = moveInitiative(rows, activeId, overId);
    if (!next) return;
    saving.current = true; setPreview(next);
    try {
      await apiPut(`/api/project/goals/${encodeURIComponent(projectId)}/initiatives/order`, {
        expected_ids: rows.map((row) => row.id), ids: next.map((row) => row.id),
      });
      await refresh();
    } catch (error) {
      onNotice(err(`子项排序保存失败：${error.message}`));
    } finally { saving.current = false; setPreview(null); }
  };
  const columns = [...initiativeColumns({ openInitiative }), {
    title: '操作', key: 'actions', width: 100, fixed: 'right',
    render: (_, record) => <SummaryHistory compact collection="initiatives" record={record} refresh={refresh} />,
  }];
  return <div>
    <Typography.Paragraph type="secondary">{preview ? '正在保存子项顺序…' : canEdit ? '拖动左侧手柄调整子项顺序' : '子项按已保存的顺序展示'}</Typography.Paragraph>
    <ProjectTable label="项目内子项表格" viewKey={`project:${projectId}:initiatives`} scroll={{ x: 1964 }}
      rows={preview || rows} columns={columns} pagination={false} rowSorting={{ disabled: !canEdit || !!preview, onMove: move }}
      onRowClick={(row) => openInitiative(row.id)} locale={{ emptyText: '这个项目还没有子项' }} />
  </div>;
}
