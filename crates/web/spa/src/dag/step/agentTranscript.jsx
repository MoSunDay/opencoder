// agent step record: the child session's frames folded into TUI turns by
// useStepStream, rendered with the console-wide TranscriptView. The terminal
// `step_finished` receipt wins over the live transcript status so a settled
// step reads as done/error even when the session never sent its own `done`.
import { Tag } from 'antd';
import { TranscriptView } from '../../transcript.jsx';

const CONNECTION = { connecting: '加载记录中', open: '已连接', live: '实时', reconnecting: '重连中', closed: '已结束', failed: '连接失败' };

export function AgentTranscript({ transcript, finished, connection }) {
  return <div className="execution-logs">
    <Tag>{CONNECTION[connection] || '连接中'}</Tag>
    <TranscriptView
      turns={transcript?.turns || []}
      usage={transcript?.usage || null}
      status={finished?.status || transcript?.status}
      error={finished?.error || transcript?.error}
      emptyText="该步骤还没有会话记录"
    />
  </div>;
}
