import { Typography } from 'antd';
import dayjs from 'dayjs';
import { Markdown } from '../markdown.jsx';
import { SummaryHistory } from './history.jsx';
import { ContentSection } from './section.jsx';

const summaryTime = (record) => record.summary_updated_at == null ? '尚未更新' : dayjs(record.summary_updated_at).format('YYYY-MM-DD HH:mm');

export function CurrentSummary({ record, collection, refresh }) {
  return <ContentSection key={`${collection}:${record?.id}`} title="现状" extra={<>
      <Typography.Text type="secondary">现状更新时间：{summaryTime(record || {})}</Typography.Text>
      {record?.id && collection && <SummaryHistory key={`${collection}:${record.id}`} collection={collection} record={record} refresh={refresh} />}
    </>}>
    {record?.current_summary ? <Markdown text={record.current_summary} /> : <Typography.Paragraph type="secondary">暂无现状</Typography.Paragraph>}
  </ContentSection>;
}
