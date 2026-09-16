import {useState} from 'react';
import {Alert, Button, Card, Collapse, Input, Select, Space, Switch, Typography} from 'antd';
import {Markdown} from '../project/markdown.jsx';
import {ResourceFiles} from './resourceFiles.jsx';
import {isDirty, PARTS, textContent, updateText} from './resourceModel.js';

export function ResourceTab({cat,label,entry,onEdit,onSave,onWorking}) {
  const [version,setVersion] = useState(); const [preview,setPreview] = useState(false);
  if (!entry) return <Card loading/>;
  if (entry.loadError) return <Alert type="error" showIcon title={`读取 ${label} 失败：${entry.loadError}`} description="为防止覆盖原内容，保存已禁用。请刷新重试。"/>;
  const {view,draft,saving,uploading,error,saved} = entry;
  const readOnly = view.read_only || saving || uploading;
  const invalidPrompt = cat === 'prompts' && !PARTS.some(({path}) => textContent(draft[path])?.trim());
  return <div>
    <Space wrap style={{marginBottom:12}}>
      <Typography.Text>{view.baseline.resource ? `当前 v${view.baseline.version}` : view.builtin_prompt ? '内置定义' : '未配置'}</Typography.Text>
      {view.read_only ? <Typography.Text>内置 Agent · 只读</Typography.Text> : <Button type="primary" loading={saving}
        disabled={uploading || invalidPrompt || !isDirty(entry)} onClick={() => onSave()}>保存</Button>}
      {isDirty(entry) && <Typography.Text type="warning">未保存</Typography.Text>}
      {saved && <Typography.Text type="success">已保存</Typography.Text>}
    </Space>
    {error && <Alert type="error" showIcon title={`保存失败：${error}`} description="草稿已保留；版本冲突时请核对后刷新。"/>}
    {view.builtin_prompt && <Card size="small" title="实际 Prompt"><Markdown text={view.builtin_prompt}/></Card>}
    {view.tool_filter && <Alert type="info" title={`工具限制：${view.tool_filter === 'All' ? '全部工具' : (view.tool_filter.Allow || []).join('、')}`}/>}
    {cat === 'prompts' ? <>
      {(!view.read_only || Object.keys(draft).length > 0) && <>
        <Space style={{marginBottom:12}}>预览 <Switch aria-label="Prompt 预览" checked={preview} onChange={setPreview}/></Space>
        {PARTS.map(({path,label}) => <Card size="small" title={label} key={path} style={{marginBottom:12}}>
          {preview ? <Markdown text={textContent(draft[path]) || ''}/> : <Input.TextArea rows={5} aria-label={`prompt-${path.split('.')[0]}`}
            disabled={readOnly} value={textContent(draft[path]) ?? ''} onChange={event => onEdit(updateText(draft,path,event.target.value))}/>}
        </Card>)}
        </>}
    </> : <ResourceFiles cat={cat} files={draft} onWorking={onWorking} onChange={onEdit} readOnly={readOnly} onSave={() => onSave()}/>}
    {!!view.versions.length && <Collapse style={{marginTop:12}} items={[{key:'history',label:'历史版本',children:<Space wrap>
      <Select aria-label={`history-${cat}`} placeholder="选择历史版本" value={version} style={{minWidth:150}} onChange={setVersion}
        options={[...view.versions].reverse().map(value => ({value,label:`v${value}`}))}/>
      {!view.read_only && <Button disabled={!version || saving || uploading} onClick={() => onSave(version)}>恢复为新版本</Button>}
      <Typography.Text type="secondary">恢复只影响当前 Agent。</Typography.Text>
    </Space>}]} />}
  </div>;
}
