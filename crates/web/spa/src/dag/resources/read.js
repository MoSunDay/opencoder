export function readPools(value) {
  if (!Array.isArray(value?.pools) || value.pools.some((pool) => typeof pool?.name !== 'string' || !Number.isInteger(pool.current))) throw new Error('二进制资源列表格式错误');
  return value.pools;
}

export function readHistory(value) {
  if (typeof value?.name !== 'string' || !Number.isInteger(value.current) || !Array.isArray(value.history) || value.history.some((version) => !Number.isInteger(version?.version))) throw new Error('二进制版本历史格式错误');
  return value;
}
