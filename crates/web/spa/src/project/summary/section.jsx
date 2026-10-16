import { Button, Space, Typography } from 'antd';
import { DownOutlined, RightOutlined } from '@ant-design/icons';
import { useId, useState } from 'react';

export function ContentSection({ title, extra, children }) {
  const [expanded, setExpanded] = useState(false);
  const contentId = useId();
  return <section aria-label={title}>
    <Space wrap style={{ marginBottom: 8 }}>
      <Typography.Title level={5} style={{ margin: 0 }}>{title}</Typography.Title>
      <Button type="text" size="small" icon={expanded ? <DownOutlined /> : <RightOutlined />}
        aria-label={`${expanded ? '收起' : '展开'}${title}`} aria-expanded={expanded} aria-controls={contentId}
        onClick={() => setExpanded((value) => !value)}>{expanded ? '收起' : '展开'}</Button>
      {extra}
    </Space>
    <div id={contentId} hidden={!expanded}>{children}</div>
  </section>;
}
