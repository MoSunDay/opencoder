import { forwardRef, useState } from 'react';
import { Input, Segmented, Typography } from 'antd';
import { Markdown } from '../markdown.jsx';

export const MarkdownInput = forwardRef(function MarkdownInput({ value = '', onChange, id, label = 'Markdown 正文', disabled, placeholder, defaultMode = 'edit', showLabel = false, minRows = 8, maxRows = 28 }, ref) {
  const [mode, setMode] = useState(defaultMode);
  return <div className="project-markdown-input">
    <div className="project-markdown-toolbar">
      {showLabel && <label htmlFor={id}>{label}<Typography.Text type="secondary">支持 Markdown</Typography.Text></label>}
      <Segmented size="small" aria-label={`${label}显示方式`} value={mode} onChange={setMode} options={[
        { label: '预览', value: 'preview' }, { label: '编辑', value: 'edit' },
      ]} />
    </div>
    <div hidden={mode !== 'edit'}><Input.TextArea ref={ref} id={id} aria-label={label} value={value}
      disabled={disabled} onChange={(event) => onChange?.(event.target.value)} autoSize={{ minRows, maxRows }} placeholder={placeholder} /></div>
    {mode === 'preview' && <div aria-label={`${label}预览`} className="project-markdown-preview"><Markdown text={value} /></div>}
  </div>;
});
