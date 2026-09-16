// dagPanel.jsx — 菜单页「DAG 工作流」: two tabs over the /api/dag endpoints.
//   定义 — defs table + create/edit drawer + dispatch (defsTab.jsx)
//   运行 — polled runs table + live run detail (runsTable.jsx / runDetail.jsx)
// A dispatch in the 定义 tab jumps to the 运行 tab focused on the new run.

import { Tabs } from 'antd';
import { useCallback, useState } from 'react';
import { DefsTab } from './dag/defsTab.jsx';
import { RunsTable } from './dag/runsTable.jsx';

export function DagPanel({ onNotice }) {
  const [tab, setTab] = useState('defs');
  const [focusRunId, setFocusRunId] = useState(''); // run id to open in detail
  const [refreshSignal, setRefreshSignal] = useState(0); // bumps a table reload

  const onDispatched = useCallback((runId) => {
    if (runId) {
      setFocusRunId(runId);
    } else {
      setRefreshSignal((n) => n + 1);
    }
    setTab('runs');
  }, []);

  const onDetailClosed = useCallback(() => setFocusRunId(''), []);

  return (
    <Tabs
      activeKey={tab}
      onChange={setTab}
      items={[
        {
          key: 'defs',
          label: '定义',
          children: <DefsTab onNotice={onNotice} onDispatched={onDispatched} />,
        },
        {
          key: 'runs',
          label: '运行',
          children: (
            <RunsTable
              onNotice={onNotice}
              refreshSignal={refreshSignal}
              focusRunId={focusRunId}
              onDetailClosed={onDetailClosed}
            />
          ),
        },
      ]}
    />
  );
}
