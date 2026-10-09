import { Alert, Button, Form, Modal } from 'antd';
import { useId, useRef, useState } from 'react';

// Mount once per editing session so cancel/reopen always starts with fresh fields.
export function AdminFormModal({ title, initialValues, submitLabel, onSubmit, onClose, children }) {
  const id = useId();
  const submitting = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const submit = async (values) => {
    if (submitting.current) return;
    submitting.current = true;
    setBusy(true); setError('');
    try { await onSubmit(values); }
    catch (failure) { setError(failure.message); }
    finally { submitting.current = false; setBusy(false); }
  };
  return <Modal open title={title} width={480} destroyOnHidden keyboard={!busy}
    mask={{ closable: false }} closable={!busy} onCancel={() => { if (!busy) onClose(); }}
    footer={<>
      <Button disabled={busy} onClick={onClose}>取消</Button>
      <Button type="primary" htmlType="submit" form={id} loading={busy}>{submitLabel}</Button>
    </>}>
    {error && <Alert className="oc-admin-feedback" type="error" showIcon title={error} />}
    <Form id={id} className="oc-admin-form" layout="vertical" initialValues={initialValues}
      disabled={busy} onFinish={submit} requiredMark="optional">
      {children}
    </Form>
  </Modal>;
}
