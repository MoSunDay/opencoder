import { Modal } from 'antd';
import { useRef } from 'react';
import { useDraftGuard } from '../../ontology/navigation/DraftGuard';

export function useCloseGuard({ dirty, busy, onClose, active = true }) {
  useDraftGuard(active && dirty, active && busy);
  const [modal, holder] = Modal.useModal();
  const confirming = useRef(false);
  const close = () => {
    if (busy || confirming.current) return;
    if (!dirty) { onClose(); return; }
    confirming.current = true;
    modal.confirm({ title: '放弃未保存的内容？', content: '任务修改和评论草稿尚未保存。', okText: '放弃修改', cancelText: '继续编辑',
      onOk: () => { confirming.current = false; onClose(); }, onCancel: () => { confirming.current = false; } });
  };
  return [close, holder];
}
