import { Progress, Typography } from 'antd';
export function TodoProgress({ progress }) {
  const { total = 0, done = 0 } = progress || {};
  const percent = total ? Math.round(done * 100 / total) : 0;
  return <div className="project-progress"><Progress percent={percent} format={(value) => `${value}%`} size="small" /><Typography.Text type="secondary">{done}/{total}</Typography.Text></div>;
}
