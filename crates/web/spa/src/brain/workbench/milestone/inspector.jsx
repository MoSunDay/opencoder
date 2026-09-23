import { Button, Input, Select, Typography } from 'antd';
import { capabilityId } from '../scheduler/model.js';
export function MilestoneInspector({ node, capabilities, onChange, onDelete, layers, onLayer }) {
  if (!node) return <aside className="brain-milestone-inspector"><h3>配置方法论</h3><p>点击里程碑配置目标与能力。同层同时推进，整层执行结束后大脑统一判断下一步。</p><p>拖动连接点设置反思回退路径。</p></aside>;
  return <aside className="brain-milestone-inspector" aria-label="里程碑配置">
    <h3>里程碑配置</h3>
    <label>名称<Input aria-label="里程碑名称" value={node.title} maxLength={120} onChange={(e) => onChange({ title: e.target.value })} /></label>
    <label>所属层<Select aria-label="所属层" value={node.layer} options={Array.from({ length: layers }, (_, i) => ({ value: i + 1, label: `第 ${i + 1} 层` }))} onChange={onLayer} /></label>
    <label>目标<Input.TextArea aria-label="里程碑目标" rows={3} maxLength={4096} value={node.objective} onChange={(e) => onChange({ objective: e.target.value })} /></label>
    <label>达成标准<Input.TextArea aria-label="达成标准" rows={3} maxLength={4096} value={node.success_criteria} onChange={(e) => onChange({ success_criteria: e.target.value })} /></label>
    <label>挂载能力<Select aria-label="挂载能力" mode="multiple" showSearch optionFilterProp="label" value={node.capability_ids} onChange={(capability_ids) => onChange({ capability_ids })} options={capabilities.map((c) => ({ value: capabilityId(c), label: `${c.kind} · ${c.summary || c.target} · ${c.version}` }))} /></label>
    {node.capability_ids.map((id) => { const c = capabilities.find((item) => capabilityId(item) === id); return <section key={id}><Typography.Text strong>{c?.summary || c?.target || id}</Typography.Text><p>输入：{c?.input_desc || '能力不可用'}</p><p>输出：{c?.output_desc || '能力不可用'}</p>{!!c?.required_inputs?.length && <p>必填输入：{c.required_inputs.join('、')}</p>}</section>; })}
    <Button danger onClick={onDelete}>删除里程碑及关联连线</Button>
  </aside>;
}
