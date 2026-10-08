import { Alert, Form, Modal } from "antd";
import type { FormInstance, ModalProps } from "antd";
import { cloneElement, useId, useState } from "react";
import type { ReactElement, ReactNode } from "react";
import { useDraftGuard } from "../navigation/DraftGuard";

type Props<T> = {
  title: ReactNode; trigger: ReactElement; initialValues?: Partial<T>; form?: FormInstance<T>;
  onFinish: (values: T) => Promise<boolean | void>; children: ReactNode;
  onOpenChange?: (open: boolean) => void;
  modalProps?: Pick<ModalProps, "destroyOnHidden">;
  submitter?: { submitButtonProps?: { disabled?: boolean } };
};
export function ModalForm<T extends object = Record<string, any>>({ title, trigger, initialValues, form: suppliedForm, onFinish, children, modalProps, submitter, onOpenChange }: Props<T>) {
  const [form] = Form.useForm<T>(suppliedForm);
  const formId = useId();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [dirty, setDirty] = useState(false);
  const guard = useDraftGuard(open && dirty, open && busy);
  const submit = async (values: T) => {
    setBusy(true); setError("");
    try { if (await onFinish(values) !== false) { setDirty(false); setOpen(false); onOpenChange?.(false); form.resetFields(); } }
    catch (failure) { setError(failure instanceof Error ? failure.message : "保存失败"); }
    finally { setBusy(false); }
  };
  return <>{cloneElement(trigger, { onClick: (event: React.MouseEvent) => {
    trigger.props.onClick?.(event); setError(""); setDirty(false); form.resetFields(); setOpen(true); onOpenChange?.(true);
  } })}<Modal forceRender {...modalProps} title={title} open={open} confirmLoading={busy}
    maskClosable={false} closable={!busy} keyboard={!busy}
    okButtonProps={submitter?.submitButtonProps} onOk={() => { if (!busy) form.submit(); }}
    onCancel={() => guard.runLocal(() => { setDirty(false); setOpen(false); onOpenChange?.(false); })}>
    {error && <Alert title={error} type="error" showIcon style={{ marginBottom: 16 }} />}
    <Form name={formId} form={form} layout="vertical" initialValues={initialValues} onValuesChange={() => setDirty(true)} onFinish={submit}>{children}</Form>
  </Modal></>;
}
