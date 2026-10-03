import type { Entity, EntityType } from "../types";

export const searchableOptions = {
  showSearch: true,
  filterOption: (input: string, option?: { searchText?: string }) =>
    (option?.searchText ?? "").toLocaleLowerCase().includes(input.trim().toLocaleLowerCase()),
};
export const searchableLabels = { showSearch: true, optionFilterProp: "label" };

export function entityTypeOptions(types: EntityType[]) {
  return types.filter((type) => !type.is_deleted).map((type) => ({
    label: type.name,
    value: type.id,
    searchText: `${type.name} ${type.type_key} ${type.id}`,
  }));
}

export function entityOptions(entities: Entity[], types: EntityType[]) {
  const activeTypes = new Map(types.filter((type) => !type.is_deleted).map((type) => [type.id, type]));
  return entities.filter((entity) => !entity.is_deleted && activeTypes.has(entity.entity_type_id)).map((entity) => {
    const type = activeTypes.get(entity.entity_type_id)!;
    return { label: entity.name, value: entity.id, searchText: `${entity.name} ${type.name} ${type.type_key} ${entity.id}` };
  });
}
