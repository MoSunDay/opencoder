// 执行明细内嵌大脑运行视图：复用工作台 BrainRunBody（useBrainRun + 步骤列表 +
// PlanCanvas + Inspector + Timeline），但不写 brain_run URL 参数——明细抽屉的
// 位置语义由抽屉自身承载，不污染浏览器地址。run id 即执行 id（/api/brain/runs/:id）。
import { BrainRunBody } from '../../brain/workbench/run.jsx';
import '../../brain/workbench/style.css';

export function BrainRunEmbed({ id, onNotice }) {
  return <BrainRunBody id={id} onNotice={onNotice} />;
}
