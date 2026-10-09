import { Sender } from '@ant-design/x';
import { Alert } from 'antd';
import { useCanEdit } from '../ui/permissions.jsx';

// Shared by the Agent page and embedded project conversations.
export function ConversationInput(props) {
  const canEdit = useCanEdit();
  if (!canEdit) return <Alert type="info" title="只读模式：可以查看对话和执行过程。" />;
  return <Sender {...props} />;
}
