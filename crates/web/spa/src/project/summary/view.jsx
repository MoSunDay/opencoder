import { Space, Typography } from 'antd';
import dayjs from 'dayjs';
import { Markdown } from '../markdown.jsx';
import { TableText } from '../views/projectTable.jsx';
import { SummaryHistory } from './history.jsx';

const summaryTime = (record) => record.summary_updated_at == null ? '尚未更新' : dayjs(record.summary_updated_at).format('YYYY-MM-DD HH:mm');

export const summaryColumns = [
  { title: '当前概述', key: 'current_summary', width: 320, searchValue: (row) => row.current_summary, render: (_, row) => <TableText>{row.current_summary || '暂无概述'}</TableText> },
  { title: '概述更新时间', key: 'summary_updated_at', width: 170, kind: 'date', searchValue: (row) => row.summary_updated_at, render: (_, row) => <TableText>{summaryTime(row)}</TableText> },
];

export function CurrentSummary({ record, collection, refresh }) {
  return <section aria-label="当前概述">
    <Space wrap style={{ marginBottom: 8 }}>
      <Typography.Title level={5} style={{ margin: 0 }}>当前概述</Typography.Title>
      <Typography.Text type="secondary">概述更新时间：{summaryTime(record || {})}</Typography.Text>
      {record?.id && collection && <SummaryHistory key={`${collection}:${record.id}`} collection={collection} record={record} refresh={refresh} />}
    </Space>
    {record?.current_summary ? <Markdown text={record.current_summary} /> : <Typography.Paragraph type="secondary">暂无概述</Typography.Paragraph>}
  </section>;
}
