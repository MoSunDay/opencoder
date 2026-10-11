import { Alert, Button, Spin } from 'antd';
import { PageShell } from '../../shell/pageShell.jsx';
import { useOverview } from '../useOverview.js';
import { ProjectViewState } from '../views/viewState.jsx';
import { TagsTab } from './tab.jsx';
import '../project.css';

export function ProjectTagsPanel({ onNotice }) {
  const { overview, loading, refresh, error } = useOverview({ onNotice });
  return <ProjectViewState><PageShell page="projectTags">
    {error && <Alert type="error" showIcon title={error} action={<Button onClick={refresh}>重试</Button>} />}
    <Spin spinning={loading}>{overview && <TagsTab overview={overview} refresh={refresh} onNotice={onNotice} />}</Spin>
  </PageShell></ProjectViewState>;
}
