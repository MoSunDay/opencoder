import { Button, Card, Col, Divider, Form, Input, InputNumber, Row, Select, Space, Typography } from 'antd';
const { TextArea } = Input;
const { Text } = Typography;
const MAX_ATTEMPTS_MIN = 1;
export function TodoForm({form,saving,allIds,todosWatch,agentOptions,onChange}) {
 return (
        <Form form={form} layout="vertical" disabled={saving} onValuesChange={onChange}>
          <Row gutter={12}>
            <Col span={12}>
              <Form.Item name="name" label="名称" rules={[{ required: true, message: '请输入名称' }]}>
                <Input placeholder="工作流名称" />
              </Form.Item>
            </Col>
          </Row>
          <Form.Item name="objective" label="目标（objective）">
            <TextArea rows={2} placeholder="这个工作流要达成什么" />
          </Form.Item>
          <Form.Item label="约束（constraints）" style={{ marginBottom: 8 }}>
            <Form.List name="constraints">
              {(fields, { add, remove }) => (
                <>
                  {fields.map((f) => (
                    <Space key={f.key} style={{ display: 'flex', marginBottom: 4 }} align="baseline">
                      <Form.Item name={f.name} noStyle>
                        <Input placeholder="约束，如：不得修改 crates/core" style={{ width: 480 }} />
                      </Form.Item>
                      <Button type="link" danger onClick={() => remove(f.name)}>删除</Button>
                    </Space>
                  ))}
                  <Button type="dashed" onClick={() => add('')} style={{ width: 200 }}>+ 添加约束</Button>
                </>
              )}
            </Form.List>
          </Form.Item>
          <Divider titlePlacement="left" plain>TODO 列表</Divider>
          <Form.List name="todos">
            {(fields, { add, remove }) => (
              <>
                {fields.map((f) => {
                  const row = (todosWatch || [])[f.name] || {};
                  const depOptions = allIds.filter((id) => id !== row.id).map((id) => ({ value: id, label: id }));
                  return (
                    <Card key={f.key} size="small" style={{ marginBottom: 12 }}
                      title={`TODO #${f.name + 1}`}
                      extra={<Button type="link" danger onClick={() => remove(f.name)}>删除</Button>}
                    >
                      <Form.Item name={[f.name, '_source_id']} hidden><Input /></Form.Item>
                      <Row gutter={12}>
                        <Col span={6}>
                          <Form.Item name={[f.name, 'id']} label="ID" rules={[{ required: true, message: '请输入 ID' }]}>
                            <Input placeholder="t1" />
                          </Form.Item>
                        </Col>
                        <Col span={9}>
                          <Form.Item name={[f.name, 'title']} label="标题" rules={[{ required: true, message: '请输入标题' }]}>
                            <Input />
                          </Form.Item>
                        </Col>
                        <Col span={5}>
                          <Form.Item name={[f.name, 'agent']} label="agent">
                            <Select options={agentOptions} />
                          </Form.Item>
                        </Col>
                        <Col span={4}>
                          <Form.Item name={[f.name, 'max_attempts']} label="最大尝试">
                            <InputNumber min={MAX_ATTEMPTS_MIN} style={{ width: '100%' }} />
                          </Form.Item>
                        </Col>
                        <Col span={24}>
                          <Form.Item name={[f.name, 'depends_on']} label="依赖（depends_on）">
                            <Select mode="multiple" options={depOptions} placeholder="可多选其它 TODO 的 id" />
                          </Form.Item>
                        </Col>
                        <Col span={12}>
                          <Form.Item name={[f.name, 'requirement_background']} label="需求背景">
                            <TextArea rows={3} />
                          </Form.Item>
                        </Col>
                        <Col span={12}>
                          <Form.Item name={[f.name, 'instructions']} label="执行说明">
                            <TextArea rows={3} />
                          </Form.Item>
                        </Col>
                        <Col span={24}>
                          <Form.Item name={[f.name, 'criteria']} label="验收标准（acceptance.criteria）">
                            <TextArea rows={2} />
                          </Form.Item>
                        </Col>
                      </Row>
                    </Card>
                  );
                })}
                <Button type="dashed" onClick={() => add({ id: '', title: '', agent: 'act', depends_on: [], max_attempts: 3, requirement_background: '', instructions: '', criteria: '' })} style={{ width: 200 }}>
                  + 添加 TODO
                </Button>
              </>
            )}
          </Form.List>
          <Divider />
          <Text type="secondary">
            提示：acceptance.required_tool_calls 等低频字段请切换到「画布」（选中节点后在右侧
            Inspector 编辑）或「JSON 源码」模式编辑。
          </Text>
        </Form>
 );
}
