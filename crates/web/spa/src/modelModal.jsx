// modelModal.jsx — model picker for the chat toolbar's 模型 button and the
// /model slash command. GET /api/models lists configured ids (default first),
// POST /api/sessions/:id/model {value} switches (409 while a drain runs —
// surfaced through onNotice). `persist_default` is deliberately omitted:
// switching is per-session here, like the TUI's /model. The endpoint
// broadcasts `model_switched`, so the transcript reflects the change by itself.

import { Button, Modal, Radio, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet, apiPost } from './api.js';

const { Text } = Typography;

export function ModelModal({ open, sessionId, onClose, onNotice }) {
  const [models, setModels] = useState([]);
  const [sel, setSel] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) {
      return;
    }
    let alive = true;
    setModels([]);
    setSel('');
    apiGet('/api/models').then((j) => {
      if (!alive) {
        return;
      }
      const list = (j && j.models) || [];
      setModels(list);
      setSel(j && j.default && list.includes(j.default) ? j.default : (list[0] || ''));
    }).catch(() => {
      // Catalog unavailable — the empty hint renders, retry on reopen.
    });
    return () => {
      alive = false;
    };
  }, [open]);

  const submit = async () => {
    if (!sessionId || !sel) {
      return;
    }
    setBusy(true);
    try {
      await apiPost('/api/sessions/' + encodeURIComponent(sessionId) + '/model', { value: sel });
      if (onNotice) {
        onNotice('模型已切换: ' + sel);
      }
      onClose();
    } catch (e) {
      if (onNotice) {
        onNotice('切换模型失败: ' + ((e && e.message) || ''));
      }
    }
    setBusy(false);
  };

  return (
    <Modal
      title="切换模型"
      open={open}
      onCancel={onClose}
      footer={[
        <Button key="cancel" onClick={onClose}>取消</Button>,
        <Button key="ok" type="primary" disabled={!sel || busy} onClick={submit}>确定</Button>,
      ]}
    >
      {models.length === 0 ? <Text type="secondary">暂无可用模型</Text> : (
        <Radio.Group value={sel} onChange={(e) => setSel(e.target.value)}>
          {models.map((m) => (
            <Radio key={m} value={m} style={{ display: 'block', padding: '2px 0' }}>{m}</Radio>
          ))}
        </Radio.Group>
      )}
    </Modal>
  );
}
