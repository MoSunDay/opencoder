import { Form, Input, Select } from 'antd';
import { PeopleFields } from '../people/fields.jsx';
import { groupOptions, searchSelect } from '../model/relations.js';
import { effectiveTags } from '../model/catalog.js';
import { LANES } from '../model/board.js';
import { MarkdownInput } from './markdownInput.jsx';

export function TodoTaskFields({ creating = false }) {
  return <div className="project-todo-field-grid">
    <div className="project-task-heading project-todo-field-full">
      <Form.Item name="title" label="标题" rules={[{ required: true, whitespace: true, message: 'TODO 标题不能为空' }]}>
        <Input aria-label={creating ? '标题' : 'TODO 标题'} placeholder="简明描述要完成的任务" />
      </Form.Item>
      <Form.Item name="board_status" label="状态"><Select aria-label="TODO 看板列" options={LANES.map(([value, label]) => ({ value, label }))} /></Form.Item>
    </div>
    <Form.Item name="draft" className="project-todo-field-full">
      <MarkdownInput label="关键信息" showLabel defaultMode={creating ? 'edit' : 'preview'} placeholder="记录任务目标、当前进展、已确认结论、卡点、下一步和资料位置，方便随时交接，支持 Markdown" />
    </Form.Item>
  </div>;
}

export function TodoSettingsFields({ overview }) {
  const form = Form.useFormInstance();
  const selected = Form.useWatch('initiative_id', { form, preserve: true });
  return <div className="project-todo-field-grid">
    <Form.Item name="initiative_id" label="所属子项"><Select {...searchSelect} aria-label="所属子项" placeholder="未归属子项" options={groupOptions(overview)} /></Form.Item>
    <Form.Item name="tag_ids" label="Tag" className="project-todo-field-full"><Select aria-label="TODO Tag" mode="multiple" showSearch optionFilterProp="label" placeholder="选择标签"
      options={effectiveTags(overview, selected).map((tag) => ({ value: tag.id, label: tag.name }))} /></Form.Item>
    <PeopleFields />
  </div>;
}
