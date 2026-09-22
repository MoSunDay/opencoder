import { Alert, Breadcrumb, Button, Space, Spin } from 'antd';
import { useEffect } from 'react';
import { LayeredRunBody } from './layered/run.jsx';
import { isLayeredView } from './layered/model.js';
import { useBrainRun } from './useRun.js';
export function BrainRunBody({ id, onNotice, header = null }) {
  const { run, error, connection, refresh } = useBrainRun(id);
  return <div className="brain-run">{header}{error && <Alert type="error" showIcon title={error} action={<Button onClick={() => refresh()?.catch(() => {})}>重试</Button>} />}
    {!run ? (!error && <Spin />) : isLayeredView(run) ? <LayeredRunBody view={run} id={id} onNotice={onNotice} connection={connection} refresh={refresh} /> : <Alert type="error" title="不支持的大脑运行版本" />}
  </div>;
}
export function BrainRunView({ id, onBack, onNotice }) {
  useEffect(() => { const params = new URLSearchParams(location.search); params.set('brain_run', id); history.replaceState(null, '', `${location.pathname}?${params}${location.hash}`); }, [id]);
  return <BrainRunBody id={id} onNotice={onNotice} header={<Space wrap><Button onClick={onBack}>返回工作台</Button><Breadcrumb items={[{ title: '大脑调度' }, { title: id }]} /></Space>} />;
}
