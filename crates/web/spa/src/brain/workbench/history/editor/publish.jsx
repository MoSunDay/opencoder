import { Alert, Button, Form, Input, Modal, Space } from 'antd';
export function PublishDialog({ metadata, busy, error, onChange, onSubmit, onClose }) {
  return <Modal open title="提交计划" footer={null} onCancel={onClose} closable={!busy} maskClosable={!busy} keyboard={!busy}>
    {error && <Alert type="error" showIcon title={error} />}
    <Form layout="vertical" initialValues={metadata} disabled={busy} onValuesChange={(_, values) => onChange(values)} onFinish={onSubmit}>
      <Form.Item name="title" label="计划名称" rules={[{ required: true, whitespace: true, message: '请填写计划名称' }]}><Input aria-label="计划名称" autoFocus /></Form.Item>
      <Form.Item name="summary" label="一句话概述" rules={[{ required: true, whitespace: true, message: '请填写一句话概述' }]}><Input.TextArea aria-label="一句话概述" rows={2} /></Form.Item>
      <Space><Button type="primary" htmlType="submit" loading={busy}>确认提交</Button><Button disabled={busy} onClick={onClose}>继续编辑</Button></Space>
    </Form>
  </Modal>;
}
