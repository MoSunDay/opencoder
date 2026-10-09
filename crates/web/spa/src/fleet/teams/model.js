/// 实时成员名单：captain 永远置顶，其余按已选顺序排列，整体按 agent 去重。
const rosterOf = (captain, members) => [...new Set([captain, ...(members || [])].filter(Boolean))];

/// 选择结果的只读展示模型：队长和成员共用一套描述，队长只通过角色列区分。
/// 这层保持纯函数，避免表格渲染和表单状态互相耦合。
export const rosterRowsOf = (captain, members, agents) => rosterOf(captain, members).map((agent) => {
  const profile = (agents || []).find((item) => item.agent === agent);
  const description = (profile?.capabilities || [])
    .map((capability) => capability.summary)
    .filter(Boolean)
    .join('；') || '暂无能力画像';
  return { key: agent, name: agent, role: agent === captain ? '队长' : '成员', description };
});

