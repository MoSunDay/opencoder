import { Form, Input } from 'antd';
import { useEffect } from 'react';
export function JsonField({ label, field, value, raw, onRaw, onChange }) {
  const serialized = JSON.stringify(value, null, 2);
  useEffect(() => { if (raw && !raw.error) { try { if (JSON.stringify(JSON.parse(raw.text), null, 2) !== serialized) onRaw(field, { text: serialized, error: '' }); } catch { /* invalid text remains in the draft */ } } }, [serialized]);
  const text = raw?.text ?? JSON.stringify(value, null, 2);
  return <Form.Item label={label} validateStatus={raw?.error ? 'error' : ''} help={raw?.error}><Input.TextArea aria-label={label} value={text} autoSize={{ minRows: 2, maxRows: 10 }} spellCheck={false} onChange={(e) => {
    try { const parsed = JSON.parse(e.target.value); onChange(parsed); onRaw(field, { text: e.target.value, error: '' }); }
    catch (error) { onRaw(field, { text: e.target.value, error: error.message }); }
  }} /></Form.Item>;
}
