// The plan owns membership and content. Only transient display state lives here.
export const flowSelection = (id) => ({ type: id.startsWith('layer:') ? 'layer' : 'node', id: id.slice(id.indexOf(':') + 1) });

export function displayNodes(projected, presentation) {
  return projected.map((node) => {
    const view = presentation[node.id];
    return { ...node, selected: view?.selected ?? node.selected,
      measured: view?.width === node.width && view?.height === node.height ? view.measured : undefined,
      ...(view?.dragging ? { position: view.position || node.position, dragging: true } : {}) };
  });
}

export function changePresentation(current, changes, projected, editable) {
  const nodes = new Map(projected.map((node) => [node.id, node]));
  const next = Object.fromEntries(Object.entries(current).filter(([id]) => nodes.has(id)));
  for (const change of changes) {
    const node = nodes.get(change.id);
    if (!node) continue;
    const previous = next[change.id] || {};
    if (change.type === 'select') next[change.id] = { ...previous, selected: change.selected };
    if (change.type === 'dimensions') next[change.id] = { ...previous, width: node.width, height: node.height, measured: change.dimensions };
    if (editable && node.type === 'layer' && change.type === 'position') next[change.id] = {
      ...previous, dragging: change.dragging === true, position: change.dragging === true ? change.position || previous.position : undefined,
    };
  }
  return next;
}

export function committedPositions(changes, nodes) {
  const positions = {};
  for (const change of changes) {
    const node = nodes.find((item) => item.id === change.id);
    if (node?.type === 'layer' && change.type === 'position' && change.dragging !== true) {
      positions[flowSelection(node.id).id] = change.position || node.position;
    }
  }
  return positions;
}
