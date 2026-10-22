import type { DataNode } from "antd/es/tree";
import type { DirectoryItem, Relationship, RelationshipType } from "../../types";

export function directoryOptions(items: DirectoryItem[]) {
  const active = new Map(items.filter(item => !item.is_deleted).map(item => [item.id, item]));
  return [...active.values()].map(item => {
    const names: string[] = [], seen = new Set<string>();
    let current: DirectoryItem | undefined = item;
    while (current && !seen.has(current.id)) {
      seen.add(current.id);
      names.unshift(current.name);
      current = current.parent_id ? active.get(current.parent_id) : undefined;
    }
    return { value: item.id, label: names.join(" / ") };
  }).sort((a, b) => a.label.localeCompare(b.label));
}

export function descendantDirectories(items: DirectoryItem[], selected: string) {
  const active = items.filter(item => !item.is_deleted);
  const children = new Map<string, string[]>();
  for (const item of active) {
    if (!item.parent_id) continue;
    children.set(item.parent_id, [...(children.get(item.parent_id) ?? []), item.id]);
  }
  const found = new Set<string>(), pending = active.some(item => item.id === selected) ? [selected] : [];
  while (pending.length) {
    const id = pending.pop()!;
    if (found.has(id)) continue;
    found.add(id);
    pending.push(...(children.get(id) ?? []));
  }
  return found;
}

export function directoryMembers(items: DirectoryItem[], relations: Relationship[], types: RelationshipType[], selected: string) {
  const directories = descendantDirectories(items, selected);
  const memberships = new Set(types.filter(type => !type.is_deleted && type.is_directory_membership).map(type => type.id));
  return new Set(relations.filter(edge => !edge.is_deleted && memberships.has(edge.relationship_type_id)
    && directories.has(edge.target_entity_id)).map(edge => edge.source_entity_id));
}

// JSON roots have a null parent; normalize them before building the tree.
export function directoryTree(items: DirectoryItem[]): DataNode[] {
  const active = items.filter(item => !item.is_deleted);
  const ids = new Set(active.map(item => item.id));
  const children = new Map<string | undefined, DirectoryItem[]>();
  for (const item of active) {
    const parent = item.parent_id && ids.has(item.parent_id) ? item.parent_id : undefined;
    children.set(parent, [...(children.get(parent) ?? []), item]);
  }
  const visited = new Set<string>();
  const build = (entries: DirectoryItem[]): DataNode[] => entries.flatMap(item => {
    if (visited.has(item.id)) return [];
    visited.add(item.id);
    return [{ key: item.id, title: item.name, children: build(children.get(item.id) ?? []) }];
  });
  const roots = build(children.get(undefined) ?? []);
  return [...roots, ...build(active.filter(item => !visited.has(item.id)))];
}
