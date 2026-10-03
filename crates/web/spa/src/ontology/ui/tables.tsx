import { Card, Space, Table, Typography } from "antd";
import type { TableProps } from "antd";
import type { CSSProperties, ReactNode } from "react";

type Props<T> = TableProps<T> & { headerTitle?: ReactNode; toolBarRender?: () => ReactNode[]; search?: false; options?: false };
export function DataTable<T extends object>({ headerTitle, toolBarRender, search: _search, options: _options, ...props }: Props<T>) {
  return <><div style={{ display: "flex", flexWrap: "wrap", gap: 12, justifyContent: "space-between", marginBottom: 16 }}>
    {headerTitle && <Typography.Text strong>{headerTitle}</Typography.Text>}
    <Space wrap>{toolBarRender?.()}</Space>
  </div><Table<T> {...props} scroll={props.scroll ?? { x: "max-content" }} /></>;
}
export function Section({ children, split, colSpan, title, extra, style }: {
  children: ReactNode; split?: "vertical" | "horizontal"; colSpan?: string; title?: ReactNode; extra?: ReactNode; style?: CSSProperties;
}) {
  if (split) return <div style={{ display: "flex", flexDirection: split === "vertical" ? "row" : "column", gap: 16, minWidth: 0, ...style }}>{children}</div>;
  return <Card title={title} extra={extra} style={{ minWidth: 0, flex: colSpan ? `0 0 ${colSpan}` : "1 1 0", ...style }}>{children}</Card>;
}
