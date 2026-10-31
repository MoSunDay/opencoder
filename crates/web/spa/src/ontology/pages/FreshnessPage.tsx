import { useState } from "react";
import { Tabs } from "antd";
import { useEnv } from "../env";
import { useDraftGuard } from "../navigation/DraftGuard";
import PolicyTable from "./freshness/PolicyTable";
import ReviewQueue from "./freshness/ReviewQueue";
import RunsTable from "./freshness/RunsTable";

export default function FreshnessPage() {
  const { env, canManage } = useEnv();
  return <FreshnessContent key={env} env={env} canManage={canManage} />;
}

/// The page owns the tab state so a triggered run can focus the runs tab on
/// the fresh run; drawers stay read-only so the tab switch needs no guard.
function FreshnessContent({ env, canManage }: { env: string; canManage: boolean }) {
  const guard = useDraftGuard();
  const [tab, setTab] = useState("policies");
  const [focusRunId, setFocusRunId] = useState<string>();
  return <div className="admin-content">
    <Tabs activeKey={tab} onChange={(key) => guard.run(() => setTab(key))} items={[
      { key: "policies", label: "策略", children: <PolicyTable env={env} canManage={canManage}
        onTriggered={(runId) => { setFocusRunId(runId); setTab("runs"); }} /> },
      { key: "runs", label: "运行", children: <RunsTable env={env} canManage={canManage} focusRunId={focusRunId} /> },
      { key: "reviews", label: "评审", children: <ReviewQueue env={env} canManage={canManage} /> },
    ]} />
  </div>;
}
