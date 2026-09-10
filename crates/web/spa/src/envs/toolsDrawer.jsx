import { useEvent } from '../ui/editing/useEvent.js';
// toolsDrawer.jsx — env 行内工具抽屉：点 env 行（或工具数 Tag）打开，工具的
// 查看/添加/移除/导入全部发生在这个抽屉里（tools 跟随 env）。打开时并行拉
// GET /api/todo/envs/:name + GET /api/todo/tools；绑定变更走
// PUT /api/todo/envs/:name 的部分合并语义（只发 {tools}，description/env_vars
// 由服务端保留），成功后 onChanged 让父面板静默刷新 env 列表；导入走
// POST /api/todo/tools/import，成功后只静默重拉目录（导入不改变绑定，故不
// 触发 onChanged）。行级互斥用单个 busy 字符串（remove:/add/import:）。

import {
  Button, Drawer, Select, Space, Spin, Table, Tag, Typography,
} from 'antd';
import { useEffect, useState } from 'react';
import { apiGet, apiPost, apiPut } from '../api.js';
import { err } from '../notice.js';
import { useMessage } from '../ui/appMessage.js';
import { MONO_VAR } from '../ui/mono.js';
import { envFromContext, importableTools, shareTools } from './envModel.js';

const { Text } = Typography;

const REF_MONO = { fontFamily: MONO_VAR, fontSize: 12 };

/// 重开即重挂（key 随 open/name 变化），Session 无需处理跨 env 的残留态。
export function EnvToolsDrawer(props) {
  return <EnvToolsDrawerSession key={`${props.open}/${props.name}`} {...props} />;
}

function EnvToolsDrawerSession({
  name, open, onClose, onNotice: noticeCallback, onChanged,
}) {
  const onNotice = useEvent(noticeCallback);
  const msg = useMessage();
  const [env, setEnv] = useState(null);
  const [tools, setTools] = useState([]);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState('');
  const [pending, setPending] = useState([]);

  useEffect(() => {
    if (!open || !name) {
      return;
    }
    let alive = true;
    setLoading(true);
    Promise.all([
      apiGet(`/api/todo/envs/${encodeURIComponent(name)}`),
      apiGet('/api/todo/tools'),
    ])
      .then(([ctx, catalog]) => {
        if (!alive) {
          return;
        }
        const e = envFromContext(ctx);
        if (!e) throw new Error('env 详情格式异常');
        setEnv(e);
        setTools((catalog && catalog.tools) || []);
      })
      .catch((e) => onNotice(err('获取 env 工具详情失败: ' + (e && e.message))))
      .finally(() => {
        if (alive) {
          setLoading(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [open, name, onNotice]);

  const bound = env && Array.isArray(env.tools) ? env.tools : [];
  const share = shareTools(tools);
  const importable = importableTools(tools);
  const boundSet = new Set(bound);

  /// 绑定变更唯一出口：全量 tools 列表 PUT（服务端部分合并，只动 tools 键）。
  const putTools = async (next, busyKey) => {
    if (busy) return false;
    setBusy(busyKey);
    try {
      await apiPut(`/api/todo/envs/${encodeURIComponent(name)}`, { tools: next });
      setEnv((prev) => ({ ...(prev || {}), tools: next }));
      msg.success('已更新工具');
      if (onChanged) {
        onChanged();
      }
      return true;
    } catch (e) {
      onNotice(err('更新工具失败: ' + (e && e.message)));
      return false;
    } finally {
      setBusy('');
    }
  };

  const removeTool = (ref) => putTools(bound.filter((r) => r !== ref), `remove:${ref}`);

  const addTools = async () => {
    if (!pending.length) return;
    const done = await putTools([...new Set([...bound, ...pending])], 'add');
    if (done) {
      setPending([]);
    }
  };

  const importTool = async (t) => {
    if (busy) return;
    setBusy(`import:${t.ref}`);
    try {
      const j = await apiPost('/api/todo/tools/import', {
        agent: t.agent, version: t.version, tool: t.tool,
      });
      msg.success('已导入: ' + ((j && j.ref) || t.ref));
      // 导入只改目录不改绑定：静默重拉 tools，不点全抽屉 spinner、不 onChanged。
      try {
        const catalog = await apiGet('/api/todo/tools');
        setTools((catalog && catalog.tools) || []);
      } catch (e) {
        onNotice(err('获取工具目录失败: ' + (e && e.message)));
      }
    } catch (e) {
      onNotice(err('导入工具失败: ' + (e && e.message)));
    } finally {
      setBusy('');
    }
  };

  const addOptions = share
    .filter((t) => !boundSet.has(t.ref))
    .map((t) => ({ value: t.ref, label: t.ref }));

  const boundCols = [
    { title: 'ref', dataIndex: 'ref', key: 'ref', ellipsis: true,
      render: (v) => <Text style={REF_MONO}>{v}</Text> },
    { title: '操作', key: 'op', width: 90, render: (_, row) => (
      <Button size="small" danger loading={busy === `remove:${row.ref}`}
        disabled={!!busy && busy !== `remove:${row.ref}`}
        onClick={() => removeTool(row.ref)}>移除</Button>
    ) },
  ];

  const impCols = [
    { title: 'ref', dataIndex: 'ref', key: 'ref', ellipsis: true,
      render: (v) => <Text style={REF_MONO}>{v}</Text> },
    { title: 'agent', dataIndex: 'agent', key: 'agent', width: 110, ellipsis: true },
    { title: 'version', dataIndex: 'version', key: 'version', width: 80, ellipsis: true },
    { title: 'tool', dataIndex: 'tool', key: 'tool', ellipsis: true },
    { title: '操作', key: 'op', width: 90, render: (_, t) => (
      <Button size="small" loading={busy === `import:${t.ref}`}
        disabled={!!busy && busy !== `import:${t.ref}`}
        onClick={() => importTool(t)}>导入</Button>
    ) },
  ];

  return (
    <Drawer
      title={`工具: ${name || ''}`}
      open={open}
      onClose={() => { if (!busy) onClose(); }}
      placement="right"
      width={720}
      destroyOnHidden
    >
      <Spin spinning={loading}>
        <Space orientation="vertical" size={16} style={{ width: '100%' }}>
          <Space wrap size={8}>
            <Text strong>{name}</Text>
            <Text type="secondary">{(env && env.description) || '-'}</Text>
            <Tag>工具 {bound.length} 个</Tag>
            <Tag>变量 {Object.keys((env && env.env_vars) || {}).length} 个</Tag>
          </Space>
          <div>
            <Text strong>已绑定工具</Text>
            <Table rowKey="ref" size="small" columns={boundCols}
              dataSource={bound.map((ref) => ({ ref }))} pagination={false}
              locale={{ emptyText: '未绑定工具' }} />
          </div>
          <div>
            <Text strong>添加工具</Text>
            <div>
              <Space wrap style={{ width: '100%' }}>
                <Select mode="multiple" value={pending} options={addOptions}
                  onChange={setPending} placeholder="选择 share 中已导入的工具"
                  aria-label="env-add-tools" disabled={loading || !!busy}
                  style={{ minWidth: 260, maxWidth: '100%' }} />
                <Button type="primary" loading={busy === 'add'} disabled={!pending.length}
                  onClick={addTools}>添加</Button>
              </Space>
              <Text type="secondary" style={{ fontSize: 12, display: 'block', marginTop: 4 }}>
                只有 share 中已导入的工具可添加；可导入条目请先在下方导入。
              </Text>
            </div>
          </div>
          <div>
            <Text strong>可导入工具</Text>
            <Table rowKey="ref" size="small" columns={impCols} dataSource={importable}
              pagination={false} locale={{ emptyText: '无可导入工具' }} />
          </div>
        </Space>
      </Spin>
    </Drawer>
  );
}
