import { Tabs } from 'antd';
import { useLayoutEffect, useRef } from 'react';

// Preserve each tab's reading position in the enclosing detail scroll area.
export function ReadingTabs({ activeKey, onChange, items }) {
  const ref = useRef(null);
  const positions = useRef({});
  useLayoutEffect(() => {
    const scroller = ref.current?.closest('.brain-detail-scroll, .ant-drawer-body');
    if (scroller) scroller.scrollTop = positions.current[activeKey] || 0;
  }, [activeKey]);
  const select = (key) => {
    const scroller = ref.current?.closest('.brain-detail-scroll, .ant-drawer-body');
    if (scroller) positions.current[activeKey] = scroller.scrollTop;
    onChange(key);
  };
  return <div ref={ref}><Tabs activeKey={activeKey} onChange={select} items={items} /></div>;
}
