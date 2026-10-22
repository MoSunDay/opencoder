import { Alert, Typography } from "antd";
import type { CatalogManagement } from "../../../../types";

export default function CapabilitySource({ management }: { management?: CatalogManagement | null }) {
  if (!management) return null;
  return <Alert type="info" showIcon message="此能力由平台同步维护"
    description={<>
      <Typography.Paragraph>通过原模块入口修改能力定义，保存后会自动同步。补充说明和自定义关系可在这里维护。</Typography.Paragraph>
      {[...(management.cli ?? []), ...(management.api ?? [])].map((entry) =>
        <Typography.Paragraph key={entry} copyable style={{ overflowWrap: "anywhere", marginBottom: 4 }}>{entry}</Typography.Paragraph>)}
    </>} />;
}
