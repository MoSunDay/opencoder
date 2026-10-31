import { Alert, Button, Drawer, Grid, Space, Table, Tag, Typography } from "antd";
import dayjs from "dayjs";
import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import { api } from "../../api";
import type { FreshnessItemStatus, FreshnessRun, FreshnessRunItem, FreshnessRunStatus } from "../../types";
import { DataTable } from "../../ui";
import { useResource } from "../admin/useResource";

export const RUN_STATUS_COLORS: Record<FreshnessRunStatus, string> = { queued: "default", running: "blue", succeeded: "green", partial: "orange", failed: "red" };
export const RUN_STATUS_LABELS: Record<FreshnessRunStatus, string> = { queued: "排队中", running: "运行中", succeeded: "成功", partial: "部分成功", failed: "失败" };
const ITEM_STATUS_COLORS: Record<FreshnessItemStatus, string> = { pending: "default", dispatched: "blue", succeeded: "green", failed: "red" };
const ITEM_STATUS_LABELS: Record<FreshnessItemStatus, string> = { pending: "待执行", dispatched: "已派发", succeeded: "成功", failed: "失败" };
export const formatRunTime = (value: string) => dayjs(value).format("YYYY-MM-DD HH:mm:ss");

export const runSummary = (run: FreshnessRun) => `成功 ${run.stats.succeeded} / 失败 ${run.stats.failed} / 共 ${run.stats.total}`;

export function RunStatusTag({ status }: { status: FreshnessRunStatus }) {
  return <Tag color={RUN_STATUS_COLORS[status]}>{RUN_STATUS_LABELS[status]}</Tag>;
}

function RunDetail({ run, reload }: { run: FreshnessRun; reload: () => void }) {
  return <Space direction="vertical" size="middle" style={{ width: "100%" }}>
    <Typography.Paragraph style={{ marginBottom: 0 }}>
      运行 <Typography.Text copyable>{run.id}</Typography.Text>
      <br />策略 <Typography.Text copyable>{run.policy_id}</Typography.Text>
    </Typography.Paragraph>
    <Space wrap>
      <RunStatusTag status={run.status} />
      <Tag>{runSummary(run)}</Tag>
      <Typography.Text type="secondary">创建于 {formatRunTime(run.created_at)} · 更新于 {formatRunTime(run.updated_at)}</Typography.Text>
    </Space>
    <Table<FreshnessRunItem> rowKey="entity_id" size="small" pagination={false} dataSource={run.items}
      expandable={{ rowExpandable: (item) => Boolean(item.candidate), expandedRowRender: (item) => <ReactMarkdown
        skipHtml components={{ a: ({ href, children }) => <a href={href} target="_blank" rel="noreferrer noopener">{children}</a> }}>{item.candidate ?? ""}</ReactMarkdown> }}
      columns={[
        { title: "实体", width: 200, render: (_, item) => <Space direction="vertical" size={0}>
          <Typography.Text strong>{item.entity_name}</Typography.Text>
          <Typography.Text type="secondary" copyable style={{ fontSize: 12 }}>{item.entity_id}</Typography.Text>
        </Space> },
        { title: "执行 ID", width: 200, render: (_, item) => item.execution_id ? <Typography.Text copyable={{ text: item.execution_id }} style={{ fontSize: 12 }}>{item.execution_id}</Typography.Text> : "-" },
        { title: "状态", width: 90, render: (_, item) => <Tag color={ITEM_STATUS_COLORS[item.status]}>{ITEM_STATUS_LABELS[item.status]}</Tag> },
        { title: "错误", ellipsis: true, render: (_, item) => item.error ? <Typography.Text type="danger" ellipsis={{ tooltip: item.error }}>{item.error}</Typography.Text> : "-" },
      ]} />
    <Button onClick={() => void reload()}>刷新运行</Button>
  </Space>;
}

export default function RunsTable({ env, focusRunId }: { env: string; canManage: boolean; focusRunId?: string }) {
  const screens = Grid.useBreakpoint();
  const [page, setPage] = useState(1);
  const [selectedId, setSelectedId] = useState<string>();
  const resource = useResource(env, async () => (await api.freshnessRuns(env, { limit: 50 })).items);
  const items = resource.data ?? [];
  const selected = items.find((run) => run.id === selectedId);
  const active = items.some((run) => run.status === "queued" || run.status === "running");
  useEffect(() => { setPage(1); }, [env]);
  useEffect(() => { if (focusRunId) setSelectedId(focusRunId); }, [focusRunId]);
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => { void resource.reload(); }, 5000);
    return () => { clearInterval(timer); };
  }, [active, resource.reload]);
  return <div>
    {resource.error ? <Alert type="error" showIcon message={resource.data ? "更新失败，当前显示上次结果" : "运行加载失败"} description={resource.error}
      action={<Button onClick={() => void resource.reload()}>重试</Button>} /> : null}
    <DataTable<FreshnessRun> scroll={{ x: 860 }} headerTitle="最近运行" rowKey="id" search={false}
      loading={resource.loading} dataSource={items}
      pagination={{ current: page, pageSize: 20, onChange: setPage, showSizeChanger: false }}
      options={{ reload: () => resource.reload() }}
      columns={[
        { title: "运行", width: 220, render: (_, run) => <Space direction="vertical" size={0}>
          <Button type="link" style={{ padding: 0 }} onClick={() => setSelectedId(run.id)}>详情</Button>
          <Typography.Text type="secondary" copyable={{ text: run.id }} style={{ fontSize: 12 }}>{run.id}</Typography.Text>
        </Space> },
        { title: "状态", width: 100, render: (_, run) => <RunStatusTag status={run.status} /> },
        { title: "统计", width: 220, render: (_, run) => runSummary(run) },
        { title: "策略", width: 220, render: (_, run) => <Typography.Text type="secondary" copyable={{ text: run.policy_id }} style={{ fontSize: 12 }}>{run.policy_id}</Typography.Text> },
        { title: "创建时间", width: 180, render: (_, run) => formatRunTime(run.created_at) },
      ]} />
    <Drawer title="运行详情" open={Boolean(selected)} size={screens.md ? 720 : "100%"} onClose={() => setSelectedId(undefined)} destroyOnHidden>
      {selected ? <RunDetail run={selected} reload={resource.reload} /> : null}
    </Drawer>
  </div>;
}
