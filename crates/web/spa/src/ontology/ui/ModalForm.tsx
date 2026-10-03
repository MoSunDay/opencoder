import { Alert, Form, Modal } from "antd";
import type { FormInstance, ModalProps } from "antd";
import { cloneElement, useId, useState } from "react";
import type { ReactElement, ReactNode } from "react";

type Props<T> = {
  title: ReactNode; trigger: ReactElement; initialValues?: Partial<T>; form?: FormInstance<T>;
  onFinish: (values: T) => Promise<boolean | void>; children: ReactNode;
  modalProps?: Pick<ModalProps, "destroyOnHidden">;
  submitter?: { submitButtonProps?: { disabled?: boolean } };
};
export function ModalForm<T extends object = Record<string, any>>({ title, trigger, initialValues, form: suppliedForm, onFinish, children, modalProps, submitter }: Props<T>) {
  const [form] = Form.useForm<T>(suppliedForm);
  const formId = useId();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const submit = async (values: T) => {
    setBusy(true); setError("");
    try { if (await onFinish(values) !== false) { setOpen(false); form.resetFields(); } }
    catch (failure) { setError(failure instanceof Error ? failure.message : "保存失败"); }
    finally { setBusy(false); }
  };
  return <>{cloneElement(trigger, { onClick: (event: React.MouseEvent) => {
    trigger.props.onClick?.(event); setError(""); form.resetFields(); setOpen(true);
  } })}<Modal forceRender {...modalProps} title={title} open={open} confirmLoading={busy}
    okButtonProps={submitter?.submitButtonProps} onOk={() => form.submit()} onCancel={() => { if (!busy) setOpen(false); }}>
    {error && <Alert title={error} type="error" showIcon style={{ marginBottom: 16 }} />}
    <Form name={formId} form={form} layout="vertical" initialValues={initialValues} onFinish={submit}>{children}</Form>
  </Modal></>;
}
