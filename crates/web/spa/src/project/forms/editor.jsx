import { useEffect, useId, useRef } from 'react';
import { Alert, Button, Form, Input, Select } from 'antd';
import { PeopleSelect } from '../people/fields.jsx';
import { groupOptions, searchSelect } from '../model/relations.js';
import { effectiveTags } from '../model/catalog.js';
import { LANES } from '../model/board.js';
import { capabilityOptions } from '../execute/catalog.js';
import { TodoSection } from './section.jsx';

export function TodoEditor({ value, onChange, overview, catalog, disabled, onSave, titleError }) {
  const id = useId();
  const titleInput = useRef(null);
  useEffect(() => { if (titleError) titleInput.current?.focus(); }, [titleError]);
  const field = (key) => `${id}-${key}`;
  return <Form layout="vertical" className="project-todo-form project-todo-form--edit" disabled={disabled} onFinish={onSave}>
    <TodoSection title="任务内容" className="project-todo-section--task">
      <Form.Item label="标题" htmlFor={field('title')} required className="project-todo-field-full" validateStatus={titleError ? 'error' : undefined} help={titleError}>
        <Input ref={titleInput} id={field('title')} aria-label="TODO 标题" value={value.title} onChange={(event) => onChange('title', event.target.value)} placeholder="简明描述要完成的任务" />
      </Form.Item>
      <Form.Item label="任务说明" htmlFor={field('draft')} className="project-todo-field-full">
        <Input.TextArea id={field('draft')} aria-label="任务说明" value={value.draft} onChange={(event) => onChange('draft', event.target.value)} autoSize={{ minRows: 6, maxRows: 18 }} placeholder="写下任务要求、交付内容和验收标准" />
      </Form.Item>
    </TodoSection>
    <TodoSection title="归属与状态">
      <Form.Item label="所属专项" htmlFor={field('initiative')}>
        <Select {...searchSelect} id={field('initiative')} aria-label="所属专项" placeholder="未归属专项" value={value.groupId} onChange={(next) => onChange('groupId', next || null)} options={groupOptions(overview)} />
      </Form.Item>
      <Form.Item label="看板状态" htmlFor={field('status')}>
        <Select id={field('status')} aria-label="TODO 看板列" value={value.status} onChange={(next) => onChange('status', next)} options={LANES.map(([value, label]) => ({ value, label }))} />
      </Form.Item>
      <Form.Item label="Tag" htmlFor={field('tags')} className="project-todo-field-full">
        <Select id={field('tags')} mode="multiple" aria-label="TODO Tag" value={value.tagIds} onChange={(next) => onChange('tagIds', next)} showSearch optionFilterProp="label" placeholder="选择标签" options={effectiveTags(overview, value.groupId).map((tag) => ({ value: tag.id, label: tag.name }))} />
      </Form.Item>
    </TodoSection>
    <TodoSection title="负责人和关注人">
      <Form.Item label="负责人" htmlFor={field('owner')}><PeopleSelect id={field('owner')} label="负责人" value={value.owner} onChange={(next) => onChange('owner', next)} /></Form.Item>
      <Form.Item label="关注人" htmlFor={field('followers')}><PeopleSelect id={field('followers')} label="关注人" multiple value={value.followers} onChange={(next) => onChange('followers', next)} /></Form.Item>
    </TodoSection>
    <TodoSection title="执行设置" className="project-todo-section--execution">
      {catalog.error && <Alert className="project-todo-field-full" type="error" title={catalog.error} action={<Button onClick={catalog.reload}>重试能力列表</Button>} />}
      <Form.Item label="执行能力" htmlFor={field('capability')} className="project-todo-field-full">
        <Select id={field('capability')} aria-label="执行能力" placeholder="选择能力库中的能力" value={value.capabilityId} onChange={(next) => onChange('capabilityId', next || null)} allowClear loading={catalog.loading} options={capabilityOptions(catalog.capabilities)} showSearch optionFilterProp="label" />
      </Form.Item>
    </TodoSection>
  </Form>;
}
