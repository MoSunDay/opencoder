import { Alert, App, Button, Empty, Input, Modal, Select, Space, Tag, Typography } from "antd";
import { useEffect, useState } from "react";
import { api } from "../../api";
import type { DataLevel, FreshnessReview, FreshnessRunItem } from "../../types";
import { DataTable } from "../../ui";
import { useDraftGuard } from "../../navigation/DraftGuard";
import { useResource } from "../admin/useResource";
import LevelBadge from "./LevelBadge";
import { levelLabel } from "./levels";
import { RUN_STATUS_LABELS, formatRunTime } from "./RunsTable";

type QueueData = { items: FreshnessRunItem[]; reviews: FreshnessReview[]; levels: Record<string, DataLevel> };
const EMPTY: QueueData = { items: [], reviews: [], levels: {} };

export default function ReviewQueue({ env, canManage }: { env: string; canManage: boolean }) {
  const { message } = App.useApp();
  const [runId, setRunId] = useState<string>();
  const [rejecting, setRejecting] = useState<FreshnessRunItem>();
  const [comment, setComment] = useState("");
  const [saving, setSaving] = useState(false);
  const guard = useDraftGuard(Boolean(comment), saving);
  const runs = useResource(env, async () => (await api.freshnessRuns(env, { limit: 20 })).items);
  const runList = runs.data ?? [];
  useEffect(() => {
    if (runId || runList.length === 0) return;
    setRunId((runList.find((run) => run.stats.succeeded > 0) ?? runList[0]).id);
  }, [runId, runList]);
  const detail = useResource(`${env}:${runId ?? "none"}`, async (): Promise<QueueData> => {
    if (!runId) return EMPTY;
    const [run, reviews, entities] = await Promise.all([api.freshnessRun(env, runId), api.freshnessReviews(env, runId), api.entities(env)]);
    return {
      items: run.item.items,
      reviews: reviews.items,
      levels: Object.fromEntries(entities.items.map((entity) => [entity.id, entity.data_level ?? "draft"])),
    };
  });
  const { items = [], reviews = [], levels = {} } = detail.data ?? EMPTY;
  const reviewed = new Set(reviews.map((review) => review.entity_id));
  const candidates = items.filter((item) => item.status === "succeeded" && item.candidate);
  const names = Object.fromEntries(items.map((item) => [item.entity_id, item.entity_name]));
  const decide = async (item: FreshnessRunItem, verdict: "approve" | "reject", commentText?: string) => {
    if (!runId) return;
    setSaving(true);
    try {
      const result = await api.createFreshnessReview(env, { run_id: runId, entity_id: item.entity_id, verdict, comment: commentText ?? "" });
      message.success(verdict === "approve" ? `已通过，等级更新为 ${levelLabel(result.entity.data_level)}` : "已驳回候选");
      setRejecting(undefined); setComment("");
      await detail.reload();
    } catch (reason) { message.error(reason instanceof Error ? reason.message : "评审失败"); }
    finally { setSaving(false); }
  };
  return <Space direction="vertical" size="middle" style={{ width: "100%" }}>
    {runs.error ? <Alert type="error" showIcon message="运行加载失败" description={runs.error}
      action={<Button onClick={() => void runs.reload()}>重试</Button>} /> : null}
    {detail.error ? <Alert type="error" showIcon message={detail.data ? "更新失败，当前显示上次结果" : "评审加载失败"} description={detail.error}
      action={<Button onClick={() => void detail.reload()}>重试</Button>} /> : null}
    <Select aria-label="选择运行" style={{ minWidth: 360 }} value={runId} onChange={setRunId} placeholder="选择要评审的运行"
      options={runList.map((run) => ({ value: run.id, label: `${formatRunTime(run.created_at)} · ${RUN_STATUS_LABELS[run.status]} · 成功 ${run.stats.succeeded}` }))} />
    <DataTable<FreshnessRunItem> scroll={{ x: 820 }} headerTitle={runId ? `待评审候选 · ${runId}` : "待评审候选"}
      rowKey="entity_id" search={false} loading={detail.loading} dataSource={candidates}
      pagination={false} options={{ reload: () => detail.reload() }}
      locale={{ emptyText: <Empty description={runId ? "该运行暂无可评审的候选" : "请先选择运行"} /> }}
      columns={[
        { title: "实体", width: 200, render: (_, item) => <Space direction="vertical" size={0}>
          <Typography.Text strong>{item.entity_name}</Typography.Text>
          <Typography.Text type="secondary" copyable style={{ fontSize: 12 }}>{item.entity_id}</Typography.Text>
        </Space> },
        { title: "候选摘要", ellipsis: true, render: (_, item) => <Typography.Text type="secondary" ellipsis={{ tooltip: item.candidate ?? "" }}>{item.candidate}</Typography.Text> },
        { title: "当前等级", width: 100, render: (_, item) => <LevelBadge level={levels[item.entity_id]} /> },
        { title: "状态", width: 90, render: (_, item) => reviewed.has(item.entity_id) ? <Tag>已评审</Tag> : <Tag color="blue">待评审</Tag> },
        { title: "操作", width: 150, render: (_, item) => <Space size={0}>
          {canManage ? <Button type="link" disabled={reviewed.has(item.entity_id)} loading={saving}
            onClick={() => void decide(item, "approve")}>通过</Button> : null}
          {canManage ? <Button type="link" danger disabled={reviewed.has(item.entity_id)}
            onClick={() => { setComment(""); setRejecting(item); }}>驳回</Button> : null}
        </Space> },
      ]} />
    <DataTable<FreshnessReview> headerTitle="评审记录" rowKey="id" search={false} loading={detail.loading}
      dataSource={reviews} pagination={{ pageSize: 10, showSizeChanger: false }} options={{ reload: () => detail.reload() }}
      columns={[
        { title: "判定", width: 90, render: (_, review) => review.verdict === "approve" ? <Tag color="green">通过</Tag> : <Tag color="red">驳回</Tag> },
        { title: "实体", width: 180, ellipsis: true, render: (_, review) => names[review.entity_id] ?? review.entity_id },
        { title: "等级变化", width: 190, render: (_, review) => review.to_level === review.from_level
          ? <LevelBadge level={review.from_level} />
          : <Space size={4}><LevelBadge level={review.from_level} /><span>→</span><LevelBadge level={review.to_level} /></Space> },
        { title: "评审人", dataIndex: "actor", width: 140, ellipsis: true },
        { title: "时间", width: 180, render: (_, review) => formatRunTime(review.created_at) },
        { title: "意见", dataIndex: "comment", ellipsis: true, render: (_, review) => review.comment || "-" },
      ]} />
    <Modal title="驳回候选" open={Boolean(rejecting)} confirmLoading={saving} okText="驳回" okButtonProps={{ danger: true }} cancelText="取消"
      onOk={() => { if (rejecting) void decide(rejecting, "reject", comment.trim() || undefined); }}
      onCancel={() => guard.run(() => setRejecting(undefined))}>
      <Input.TextArea aria-label="驳回意见" placeholder="可选，说明驳回原因" value={comment} onChange={(event) => setComment(event.target.value)} />
    </Modal>
  </Space>;
}
