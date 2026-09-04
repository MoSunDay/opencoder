// brainPanel.jsx — 菜单页「项目目标」（brain 能力库）：语义搜索（POST
// /api/brain/search，distance 定位）+ 能力录入/编辑表单（POST/PUT
// /api/brain/capabilities）+ 能力列表（GET/DELETE）。页面状态全部自管，
// 不触碰 store.js；错误经 message.error 透出服务端 `error` 字段。

import {
  Button, Card, Col, Form, Input, Popconfirm, Row, Select, Space, Spin, Table, Tag, Tooltip, Typography, message,
} from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPost, apiPut } from './api.js';
import { absTime, relTime } from './format.js';

const { TextArea } = Input;
const { Paragraph } = Typography;

const K_OPTIONS = [3, 5, 10, 20, 50].map((k) => ({ value: k, label: 'top ' + k }));
const TYPE_COLORS = ['geekblue', 'purple', 'cyan', 'green', 'orange', 'magenta'];

/// 能力类型 → 稳定 Tag 颜色（字符串 hash，纯展示用途）。
function typeColor(t) {
  const s = String(t || '');
  let h = 0;
  for (let i = 0; i < s.length; i += 1) {
    h = (h * 31 + s.charCodeAt(i)) >>> 0;
  }
  return TYPE_COLORS[h % TYPE_COLORS.length];
}

function TypeTag({ value }) {
  return <Tag color={typeColor(value)}>{value || '-'}</Tag>;
}

/// 输入/输出描述的折叠呈现：两行截断，点击「展开」看全文。
function FoldDesc({ text }) {
  return (
    <Paragraph ellipsis={{ rows: 2, expandable: true, symbol: '展开' }} style={{ marginBottom: 0 }}>
      {text || <Typography.Text type="secondary">-</Typography.Text>}
    </Paragraph>
  );
}

/// 把一条 { capability, eng_inputs } 记录铺进编辑表单（编辑 / 搜索结果载入共用）。
function toFormValues(entry) {
  const c = (entry && entry.capability) || {};
  const inputs = ((entry && entry.eng_inputs) || []).map((e) => (e && e.content) || '');
  return {
    capability_type: c.capability_type || '',
    summary: c.summary || '',
    input_desc: c.input_desc || '',
    output_desc: c.output_desc || '',
    eng_inputs: inputs,
  };
}

export function BrainPanel() {
  const [form] = Form.useForm();
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [editingId, setEditingId] = useState(null);
  const [query, setQuery] = useState('');
  const [k, setK] = useState(10);
  const [hits, setHits] = useState([]);
  const [searching, setSearching] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const j = await apiGet('/api/brain/capabilities');
      setRows((j && j.capabilities) || []);
    } catch (e) {
      message.error('获取能力库失败: ' + ((e && e.message) || ''));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const search = async () => {
    const q = String(query || '').trim();
    if (!q) {
      message.warning('请输入搜索内容');
      return;
    }
    setSearching(true);
    try {
      const j = await apiPost('/api/brain/search', { query: q, k });
      setHits((j && j.hits) || []);
    } catch (e) {
      message.error('搜索失败: ' + ((e && e.message) || ''));
    } finally {
      setSearching(false);
    }
  };

  /// 编辑 / 搜索结果「编辑」共用：把记录载入表单并滚动标记为编辑态。
  const loadIntoForm = (entry) => {
    setEditingId(entry.capability.id);
    form.setFieldsValue(toFormValues(entry));
  };

  const submit = async (values) => {
    const body = {
      capability_type: values.capability_type,
      summary: values.summary,
      input_desc: values.input_desc,
      output_desc: values.output_desc,
      eng_inputs: (values.eng_inputs || []).map((s) => String(s || '').trim()).filter((s) => s.length > 0),
    };
    try {
      if (editingId) {
        await apiPut('/api/brain/capabilities/' + encodeURIComponent(editingId), body);
        message.success('能力已更新');
      } else {
        await apiPost('/api/brain/capabilities', body);
        message.success('能力已录入');
      }
      form.resetFields();
      setEditingId(null);
      await load();
    } catch (e) {
      message.error('保存失败: ' + ((e && e.message) || ''));
    }
  };

  const remove = async (id) => {
    try {
      await apiDel('/api/brain/capabilities/' + encodeURIComponent(id));
      message.success('已删除');
      await load();
    } catch (e) {
      message.error('删除失败: ' + ((e && e.message) || ''));
    }
  };

  const cancelEdit = () => {
    setEditingId(null);
    form.resetFields();
  };

  const columns = [
    { title: '类型', dataIndex: ['capability', 'capability_type'], key: 'type', width: 130, render: (v) => <TypeTag value={v} /> },
    { title: '一句话描述', dataIndex: ['capability', 'summary'], key: 'summary', ellipsis: true },
    {
      title: '工程输入',
      key: 'eng_count',
      width: 90,
      render: (_, r) => String(((r && r.eng_inputs) || []).length),
    },
    {
      title: '更新时间',
      dataIndex: ['capability', 'updated_at'],
      key: 'updated_at',
      width: 120,
      render: (ts) => (
        <Tooltip title={absTime(ts)}>
          <span>{relTime(ts)}</span>
        </Tooltip>
      ),
    },
    {
      title: '操作',
      key: 'ops',
      width: 140,
      render: (_, r) => (
        <Space>
          <Button size="small" type="link" onClick={() => loadIntoForm(r)}>编辑</Button>
          <Popconfirm
            title="删除该能力？"
            okText="确认删除"
            okButtonProps={{ danger: true }}
            cancelText="取消"
            onConfirm={() => remove(r.capability.id)}
          >
            <Button size="small" type="link" danger>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <Row gutter={[16, 16]}>
      <Col span={10}>
        <Card title="语义搜索" size="small">
          <Space.Compact style={{ width: '100%', marginBottom: 12 }}>
            <Input
              placeholder="按意图搜索能力，如：解析依赖图"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onPressEnter={search}
            />
            <Select value={k} options={K_OPTIONS} onChange={setK} style={{ width: 100 }} />
            <Button type="primary" loading={searching} onClick={search}>搜索</Button>
          </Space.Compact>
          <Spin spinning={searching}>
            {hits.length === 0 ? (
              <Typography.Text type="secondary">暂无搜索结果</Typography.Text>
            ) : (
              <div>
                {hits.map((h, i) => {
                  const c = (h && h.capability) || {};
                  const dist = typeof (h && h.distance) === 'number' ? h.distance.toFixed(4) : '-';
                  return (
                    <Card
                      key={c.id || i}
                      size="small"
                      style={{ marginBottom: 8 }}
                      title={(
                        <Space>
                          <TypeTag value={c.capability_type} />
                          <span>{c.summary}</span>
                        </Space>
                      )}
                      extra={<Tooltip title={'distance: ' + dist}><Tag>{dist}</Tag></Tooltip>}
                    >
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>输入</Typography.Text>
                      <FoldDesc text={c.input_desc} />
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>输出</Typography.Text>
                      <FoldDesc text={c.output_desc} />
                      <Button size="small" type="link" onClick={() => loadIntoForm(h)}>编辑</Button>
                    </Card>
                  );
                })}
              </div>
            )}
          </Spin>
        </Card>
      </Col>
      <Col span={14}>
        <Card
          title={editingId ? '编辑能力' : '录入能力'}
          size="small"
          extra={editingId ? <Button size="small" onClick={cancelEdit}>取消编辑</Button> : null}
        >
          <Form form={form} layout="vertical" onFinish={submit}>
            <Row gutter={12}>
              <Col span={8}>
                <Form.Item name="capability_type" label="能力类型" rules={[{ required: true, message: '请输入能力类型' }]}>
                  <Input placeholder="如：goal / constraint" />
                </Form.Item>
              </Col>
              <Col span={16}>
                <Form.Item name="summary" label="一句话描述" rules={[{ required: true, message: '请输入一句话描述' }]}>
                  <Input placeholder="这个能力做什么" />
                </Form.Item>
              </Col>
            </Row>
            <Form.Item name="input_desc" label="输入描述" rules={[{ required: true, message: '请输入输入描述' }]}>
              <TextArea rows={2} placeholder="期望的输入是什么" />
            </Form.Item>
            <Form.Item name="output_desc" label="输出描述" rules={[{ required: true, message: '请输入输出描述' }]}>
              <TextArea rows={2} placeholder="产出的结果是什么" />
            </Form.Item>
            <Form.Item label="工程输入（示例输入）" style={{ marginBottom: 8 }}>
              <Form.List name="eng_inputs">
                {(fields, { add, remove }) => (
                  <>
                    {fields.map((f) => (
                      <Space key={f.key} style={{ display: 'flex', marginBottom: 4 }} align="baseline">
                        <Form.Item
                          name={f.name}
                          noStyle
                          rules={[{ required: true, message: '请输入工程输入或删除该行' }]}
                        >
                          <Input placeholder="一条示例输入" style={{ width: 420 }} />
                        </Form.Item>
                        <Button type="link" danger onClick={() => remove(f.name)}>删除</Button>
                      </Space>
                    ))}
                    <Button type="dashed" onClick={() => add('')} style={{ width: 200 }}>+ 添加工程输入</Button>
                  </>
                )}
              </Form.List>
            </Form.Item>
            <Button type="primary" htmlType="submit">{editingId ? '保存修改' : '录入'}</Button>
          </Form>
        </Card>
        <Card title="能力列表" size="small" style={{ marginTop: 16 }}>
          <Table
            rowKey={(r) => (r.capability && r.capability.id) || ''}
            size="small"
            columns={columns}
            dataSource={rows}
            loading={loading}
            pagination={false}
            locale={{ emptyText: '暂无能力' }}
          />
        </Card>
      </Col>
    </Row>
  );
}
