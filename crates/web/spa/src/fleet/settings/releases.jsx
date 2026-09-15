import { Alert, Button, Descriptions, Modal, Space, Table, Tag } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../api.js';

const PHASE = { validated: '校验完成', warming: '预热中', ready: '已就绪', switching: '切换中', verifying: '验证中', complete: '发布完成', failed: '预热失败', rolling_back: '回滚中', rolled_back: '已回滚', migrating: '首次迁移中' };
const MODE = { active: '接收新任务', staged: '预热', retired: '承载历史任务' };

export function ReleasesModal({ onClose, onInspect }) {
  const [value, setValue] = useState(null);
  const [error, setError] = useState('');
  useEffect(() => {
    let alive = true; let timer;
    const load = async () => {
      try { const next = await apiGet('/api/admin/release'); if (alive) { setValue(next); setError(''); } }
      catch (failure) { if (alive) setError(failure.message); }
      finally { if (alive) timer = setTimeout(load, 2000); }
    };
    load();
    return () => { alive = false; clearTimeout(timer); };
  }, []);
  const release = value?.release;
  return <Modal open title="发布状态" onCancel={onClose} footer={null} width={920}>
    <Space orientation="vertical" style={{ width: '100%' }}>
      {error && <Alert type="error" title={error} />}
      {value?.enabled === false && <Alert type="info" title="平滑发布尚未启用" />}
      {release && <>
        <Descriptions size="small" column={1} items={[
          { key: 'phase', label: '阶段', children: PHASE[release.phase] || release.phase },
          { key: 'current', label: '当前版本', children: release.current || '—' },
          { key: 'candidate', label: '候选版本', children: release.candidate || '—' },
        ]} />
        {release.failure && <Alert type="error" title={release.failure} />}
        {Object.entries(release.retirement || {}).filter(([, row]) => row.failure).map(([id, row]) => <Alert key={id} type="error" title={`${id} 服务退役失败：${row.failure}`} />)}
        <Table pagination={false} rowKey={(row) => row.runtime.id} dataSource={value.host?.runtimes || []} columns={[
          { title: '版本', render: (_, row) => row.runtime.release_id },
          { title: '状态', render: (_, row) => <Tag color={row.runtime.mode === 'active' ? 'success' : 'default'}>{row.hibernated ? '休眠，访问时恢复' : MODE[row.runtime.mode] || row.runtime.mode}</Tag> },
          { title: '任务', render: (_, row) => <Space wrap>{row.remaining?.length ? row.remaining.map(([, id, phase]) => <Button type="link" key={id} onClick={() => onInspect(id)}>{id} · {phase === 'queued' ? '排队' : '执行中'}</Button>) : '无运行或排队任务'}</Space> },
          { title: '回收结果', render: (_, row) => row.collection?.error || '—' },
        ]} />
      </>}
    </Space>
  </Modal>;
}
