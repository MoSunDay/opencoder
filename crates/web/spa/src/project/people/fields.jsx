import { Alert, Button, Form, Select, Space, Typography } from 'antd';
import { useJsonQuery } from '../../ui/requests/query.js';
import { useCanEdit } from '../../ui/permissions.jsx';

function readPeople(body) {
  if (!Array.isArray(body?.people) || body.people.some((row) => typeof row.name !== 'string')) throw new Error('人员列表格式错误');
  return body.people.map(({ name }) => ({ value: name, label: name }));
}

export function PeopleSelect({ value, onChange, multiple = false, label, id }) {
  const canEdit = useCanEdit();
  const { data, loading, error, reload } = useJsonQuery('/api/project/people', readPeople);
  return <Space orientation="vertical" style={{ width: '100%' }}>
    {error && <Alert type="error" title={error} action={<Button onClick={reload}>重试人员列表</Button>} />}
    <Select id={id} aria-label={label} mode={multiple ? 'multiple' : undefined} value={value ?? (multiple ? [] : undefined)}
      onChange={(next) => onChange?.(next ?? (multiple ? [] : null))} options={data || []} loading={loading}
      disabled={!canEdit || !!error || loading ? true : undefined} showSearch optionFilterProp="label" allowClear style={{ width: '100%' }}
      placeholder={multiple ? '选择关注人' : '未指定负责人'} />
  </Space>;
}

export function PeopleFields() {
  return <>
    <Form.Item name="owner_user_name" label="负责人"><PeopleSelect label="负责人" /></Form.Item>
    <Form.Item name="follower_user_names" label="关注人"><PeopleSelect label="关注人" multiple /></Form.Item>
  </>;
}

export function PeopleSummary({ record }) {
  return <Space wrap>
    <Typography.Text>负责人：{record?.owner_user_name || '未指定'}</Typography.Text>
    <Typography.Text>关注人：{record?.follower_user_names?.join('、') || '暂无'}</Typography.Text>
  </Space>;
}

export function peopleColumns() {
  return [
    { title: '负责人', key: 'owner', kind: 'enum', width: 150, searchValue: (row) => row.owner_user_name || '未指定', render: (_, row) => row.owner_user_name || '未指定' },
    { title: '关注人', key: 'followers', kind: 'enum', width: 200, searchValue: (row) => row.follower_user_names || [], render: (_, row) => <Typography.Text ellipsis={{ tooltip: true }}>{row.follower_user_names?.join('、') || '暂无'}</Typography.Text> },
  ];
}
