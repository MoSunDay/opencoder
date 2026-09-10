import { Conversations } from '@ant-design/x';
import { Select, Spin } from 'antd';
import { dialogsToItems } from './conversationItems.js';
import { explicitNodeOptions } from './fleet/model.js';

export function DialogSidebar({
  nodes,
  nodeSel,
  onNodeChange,
  dialogs,
  activeKey,
  onActiveChange,
  onNew,
  loading,
  disabled,
}) {
  const nodeOptions = explicitNodeOptions(nodes, 'agent');

  return (
    <div
      style={{
        width: 264,
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        borderRight: '1px solid var(--oc-border)',
        paddingRight: 12,
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
            creation={{ label: '新建对话', onClick: onNew, disabled: disabled || !nodeSel }}
          />
        </Spin>
      </div>
    </div>
  );
}
