import { Button } from "antd";
import type { ReactNode } from "react";

/** Detail navigation is an action, with native keyboard and focus behavior. */
export default function InspectLink({ children, onClick }: { children: ReactNode; onClick: () => void }) {
  return <Button type="link" onClick={onClick} style={{ padding: 0, height: "auto", whiteSpace: "normal", textAlign: "left" }}>{children}</Button>;
}
