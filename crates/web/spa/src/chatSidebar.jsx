// chatSidebar.jsx — left column of the chat page (T5): node switcher on top,
// @ant-design/x Conversations list below, X-styled "新建对话" creation button
// pinned by the list. Pure presentation: every datum and callback arrives as
// a prop from ChatPanel, which keeps owning the data flow (loadDialogs /
// openDialog / resetTranscript are untouched — only their mount points move).
//
// DOM landmarks (verified in @ant-design/x 2.9 sources, es/conversations/):
//   list root    <ul class="ant-conversations">
//   item         <li class="ant-conversations-item"> (+ -active when selected)
//   item label   .ant-conversations-label
//   creation     <button class="ant-conversations-creation"> (PlusOutlined + label)

import { Conversations } from '@ant-design/x';
import { Select, Spin } from 'antd';
import { dialogsToItems } from './conversationItems.js';
import { LOCAL_NODE, LOCAL_NODE_LABEL } from './store.js';

export function DialogSidebar({
  nodes,
  nodeSel,
  onNodeChange,
  dialogs,
  activeKey,
  onActiveChange,
  onNew,
  loading,
}) {
  // Same option shape the header Select used before T5: local engine first,
  // then the fleet snapshot shared by the nodes tab.
  const nodeOptions = [{ value: LOCAL_NODE, label: LOCAL_NODE_LABEL }]
    .concat((nodes || []).map((n) => ({ value: n.id, label: n.name || n.id })));

  return (
    <div
      style={{
        width: 264,
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        borderRight: '1px solid #f0f0f0',
        paddingRight: 12,
      }}
    >
      <Select
        style={{ width: '100%', marginBottom: 12 }}
        size="small"
        value={nodeSel}
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
            creation={{ label: '新建对话', onClick: onNew }}
          />
        </Spin>
      </div>
    </div>
  );
}
