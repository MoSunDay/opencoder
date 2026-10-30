import type { Directory, Relationship, RelationshipType } from "../../types";

/** Tree node consumed by antd Tree: keys are directory uuids, the root node is synthesized. */
export type DirectoryTreeNode = { key: string; title: string; children: DirectoryTreeNode[] };

/** antd treeData for the directory hierarchy: one synthesized root plus name-sorted children. */
export const buildDirectoryTree = (items: Directory[], rootId: string): DirectoryTreeNode[] => {
  const active = items.filter((item) => !item.is_deleted);
  const names = new Map(active.map((item) => [item.id, item.name]));
  const childrenOf = new Map<string, Directory[]>();
  for (const item of active) {
    if (item.id === rootId) continue;
    const parent = item.parent_id && names.has(item.parent_id) ? item.parent_id : rootId;
    childrenOf.set(parent, [...(childrenOf.get(parent) ?? []), item]);
  }
  const build = (id: string): DirectoryTreeNode[] => (childrenOf.get(id) ?? [])
    .slice().sort((left, right) => left.name.localeCompare(right.name))
    .map((item) => ({ key: item.id, title: item.name, children: build(item.id) }));
  return [{ key: rootId, title: names.get(rootId) ?? "根目录", children: build(rootId) }];
};

/** True when a directory with the same name already exists under the given parent. */
export const siblingNameExists = (items: Directory[], parentId: string, name: string, excludeId?: string): boolean => {
  const target = name.trim();
  if (!target) return false;
  return items.some((item) => item.id !== excludeId && !item.is_deleted && item.parent_id === parentId && item.name.trim() === target);
};

/** The directory itself plus every descendant id; a directory may never move into this set. */
export const descendantIds = (items: Directory[], id: string): Set<string> => {
  const childrenOf = new Map<string, Directory[]>();
  for (const item of items) {
    if (item.is_deleted) continue;
    childrenOf.set(item.parent_id ?? "", [...(childrenOf.get(item.parent_id ?? "") ?? []), item]);
  }
  const seen = new Set<string>([id]);
  const walk = (current: string) => {
    for (const child of childrenOf.get(current) ?? []) {
      if (seen.has(child.id)) continue;
      seen.add(child.id);
      walk(child.id);
    }
  };
  walk(id);
  return seen;
};

/** The system relationship type that marks directory membership. */
export const membershipType = (types: RelationshipType[]): RelationshipType | undefined =>
  types.find((type) => type.is_directory_membership && !type.is_deleted);

/** Active membership relationships that place members directly inside the directory. */
export const directoryMembers = (relationships: Relationship[], membershipTypeId: string | undefined, directoryId: string): Relationship[] =>
  membershipTypeId
    ? relationships.filter((relation) => relation.relationship_type_id === membershipTypeId && !relation.is_deleted && relation.target_entity_id === directoryId)
    : [];

/** Directory ids the entity currently belongs to (an entity may belong to several directories). */
export const entityDirectories = (relationships: Relationship[], membershipTypeId: string | undefined, entityId: string): string[] =>
  membershipTypeId
    ? relationships
      .filter((relation) => relation.relationship_type_id === membershipTypeId && !relation.is_deleted && relation.source_entity_id === entityId)
      .map((relation) => relation.target_entity_id)
    : [];
