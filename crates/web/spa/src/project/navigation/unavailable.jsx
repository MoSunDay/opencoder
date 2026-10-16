import { Alert, Button, Spin } from 'antd';

export function UnavailableDetail({ label, overview, error, refresh }) {
  if (error) return <Alert type="error" showIcon title={`无法读取${label}`} description={error}
    action={<Button size="small" onClick={refresh}>重试</Button>} />;
  if (!overview) return <Spin description={`正在加载${label}`}><div style={{ minHeight: 80 }} /></Spin>;
  return <Alert type="warning" showIcon title={`${label}不存在或已被删除`} description="请确认分享链接是否正确，或刷新后重试。"
    action={<Button size="small" onClick={refresh}>重试</Button>} />;
}
