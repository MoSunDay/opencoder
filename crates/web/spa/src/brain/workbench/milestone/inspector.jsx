import { Button, Input, Select, Typography } from 'antd';
import { capabilityId } from '../scheduler/model.js';
import { capabilityLabel, capabilityTask } from './model.js';

export function MilestoneInspector({ plan, selection, capabilities, onLayerChange, onNodeChange, onTransitionChange, onMoveNode, onConnect, onDelete }) {
  if (!selection) return <aside className="brain-milestone-inspector"><h3>配置方法论</h3><p>点击里程碑、执行节点或层间连线，配置目标、能力与扭转条件。</p></aside>;
  const layer = selection.type === 'layer' && plan.layers.find((item) => item.layer_id === selection.id);
  const node = selection.type === 'node' && plan.nodes.find((item) => item.node_id === selection.id);
  const edge = selection.type === 'transition' && plan.transitions.find((item) => `${item.from}:${item.to}` === selection.id);
  if (layer) return <aside className="brain-milestone-inspector" aria-label="里程碑配置">
    <h3>里程碑 · 第 {plan.layers.indexOf(layer) + 1} 层</h3>
    <label>名称<Input aria-label="里程碑名称" value={layer.title} maxLength={120} onChange={(event) => onLayerChange({ title: event.target.value })} /></label>
    <label>要做什么<Input.TextArea aria-label="里程碑要做什么" rows={3} maxLength={4096} value={layer.task || ''} onChange={(event) => onLayerChange({ task: event.target.value })} /></label>
    <label>目标<Input.TextArea aria-label="里程碑目标" rows={3} maxLength={4096} value={layer.objective} onChange={(event) => onLayerChange({ objective: event.target.value })} /></label>
    <label>达成标准<Input.TextArea aria-label="里程碑达成标准" rows={3} maxLength={4096} value={layer.success_criteria} onChange={(event) => onLayerChange({ success_criteria: event.target.value })} /></label>
    <label>添加流转连线<Select aria-label="添加流转连线" placeholder="选择下一里程碑或回退目标" value={undefined} options={plan.layers.filter((item, index) => index <= plan.layers.indexOf(layer) + 1 && !plan.transitions.some((edge) => edge.from === layer.layer_id && edge.to === item.layer_id)).map((item) => ({ value: item.layer_id, label: item.title || item.layer_id }))} onChange={(to) => onConnect(layer.layer_id, to)} /></label>
    <p>本层 {plan.nodes.filter((item) => item.layer_id === layer.layer_id).length} 个执行节点并行运行。</p>
    <Button danger onClick={onDelete}>删除里程碑及其节点</Button>
  </aside>;
  if (node) {
    const capability = capabilities.find((item) => capabilityId(item) === node.capability_id);
    return <aside className="brain-milestone-inspector" aria-label="执行节点配置">
      <h3>并行执行节点</h3>
      <label>所属里程碑<Select aria-label="所属里程碑" value={node.layer_id} options={plan.layers.map((item) => ({ value: item.layer_id, label: item.title || item.layer_id }))} onChange={onMoveNode} /></label>
      <label>泛化能力<Select aria-label="绑定能力" showSearch optionFilterProp="label" value={node.capability_id || undefined} onChange={(capability_id) => onNodeChange(capabilities.find((item) => capabilityId(item) === capability_id))} options={capabilities.map((item) => ({ value: capabilityId(item), label: capabilityLabel(item) }))} /></label>
      {capability && <section><Typography.Text strong>{capabilityLabel(capability)}</Typography.Text><p>{capabilityTask(capability)}</p><p>输入：{capability.input_desc}</p><p>输出：{capability.output_desc}</p>{!!capability.required_inputs?.length && <p>必填：{capability.required_inputs.join('、')}</p>}</section>}
      <Button danger onClick={onDelete}>删除执行节点</Button>
    </aside>;
  }
  if (edge) return <aside className="brain-milestone-inspector" aria-label="层间连线配置">
    <h3>层间扭转</h3><p>{plan.layers.find((item) => item.layer_id === edge.from)?.title || edge.from} → {plan.layers.find((item) => item.layer_id === edge.to)?.title || edge.to}</p>
    <label>决策条件<Input.TextArea aria-label="扭转条件" rows={4} maxLength={1024} value={edge.condition} onChange={(event) => onTransitionChange({ condition: event.target.value })} /></label>
    <p>大脑依据执行证据判断是否走这条边；只能选择本层画出的出边。</p>
  </aside>;
  return null;
}
