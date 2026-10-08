import { Button, Drawer, Grid, Popover, Typography } from "antd";
import { useState, type ReactNode } from "react";

export default function FilterSection({ summary, children }: { summary: string; children: ReactNode }) {
  const screens = Grid.useBreakpoint();
  const [open, setOpen] = useState(false);
  if (!screens.md) return <div className="graph-filters graph-filter-summary">
    <Typography.Text type="secondary" ellipsis={{ tooltip: summary }}>{summary}</Typography.Text>
    <Button onClick={() => setOpen(true)}>调整范围</Button>
    <Drawer title="调整观测范围" open={open} onClose={() => setOpen(false)} width="100%"
      extra={<Button type="primary" onClick={() => setOpen(false)}>完成</Button>}>{children}</Drawer>
  </div>;
  return <div className="graph-filters graph-filter-summary">
    <Typography.Text type="secondary" ellipsis={{ tooltip: summary }}>{summary}</Typography.Text>
    <Popover trigger="click" placement="bottomRight" open={open} onOpenChange={setOpen} title="调整观测范围"
      content={<div style={{ width: 640, maxWidth: "calc(100vw - 48px)" }}>{children}<div style={{ textAlign: "right", marginTop: 12 }}>
        <Button type="primary" onClick={() => setOpen(false)}>完成</Button></div></div>}>
      <Button onClick={() => setOpen(true)}>调整范围</Button>
    </Popover>
  </div>;
}
