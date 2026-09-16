import { ClearOutlined, DeleteOutlined } from '@ant-design/icons';
import { Conversations } from '@ant-design/x';
import { Button, Select, Spin } from 'antd';
import { dialogsToItems } from './conversationItems.js';
import { explicitNodeOptions } from './fleet/model.js';

export function DialogSidebar({
  nodes,
  nodeSel,
  nodeKind,
  onNodeChange,
  dialogs,
  activeKey,
  onActiveChange,
  onDelete,
  onDeleteAll,
  loading,
  disabled,
}) {
  // 节点下拉按创建模式过滤可执行节点（Operator 模式 'operator'，Agent 模式
  // 'agent'）；未传 kind 时保持旧缺省，兼容既有调用。
  const nodeOptions = explicitNodeOptions(nodes, nodeKind || 'operator');

  return (
    <div
      style={{
        width: 264,
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        borderRight: '1px solid var(--oc-border)',
        paddingRight: 16,
      }}
    >
      <Select
        aria-label="执行节点"
        placeholder="请选择执行节点"
        disabled={disabled}
        style={{ width: '100%', marginBottom: 12 }}
        size="small"
        value={nodeSel || undefined}
        onChange={onNodeChange}
        options={nodeOptions}
        showSearch
        optionFilterProp="label"
      />
      <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
        <Spin spinning={loading}>
          <Conversations
            items={dialogsToItems(dialogs)}
            activeKey={activeKey}
            onActiveChange={onActiveChange}
            menu={(item) => ({
              items: [{ key: 'delete', danger: true, icon: <DeleteOutlined />, label: '删除' }],
              onClick: ({ key }) => { if (key === 'delete') onDelete?.(item.key); },
            })}
          />
        </Spin>
      </div>
      <Button
        danger
        block
        size="small"
        icon={<ClearOutlined />}
        disabled={disabled || !dialogs.length}
        onClick={onDeleteAll}
        style={{ marginTop: 12 }}
      >
        删除全部会话
      </Button>
    </div>
  );
}
