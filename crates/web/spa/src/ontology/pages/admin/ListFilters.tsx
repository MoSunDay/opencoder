import { Input, Select, Typography } from "antd";
import type { ReactNode } from "react";

export type StatusFilter = "active" | "deleted" | "all";
export function matchesRecord(item: { is_deleted: boolean }, status: StatusFilter, query: string, fields: string[]) {
  return (status === "all" || item.is_deleted === (status === "deleted"))
    && fields.join(" ").toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
}
export default function ListFilters({ query, status, count, onQuery, onStatus, children }: {
  query: string; status: StatusFilter; count: number; onQuery: (query: string) => void; onStatus: (status: StatusFilter) => void; children?: ReactNode;
}) {
  return <div className="admin-filters">
    <Input aria-label="搜索名称或标识" placeholder="搜索名称、描述或标识" allowClear value={query} onChange={(e) => onQuery(e.target.value)} />
    {children}
    <Select aria-label="记录状态" value={status} onChange={onStatus} options={[
      { value: "active", label: "有效记录" }, { value: "deleted", label: "已停用 / 已删除" }, { value: "all", label: "全部状态" },
    ]} /><Typography.Text type="secondary">匹配 {count} 条</Typography.Text>
  </div>;
}
