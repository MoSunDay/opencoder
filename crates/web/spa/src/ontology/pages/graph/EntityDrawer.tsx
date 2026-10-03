import type { Entity, EntityType } from "../../types";
import EntityDetailDrawer from "../entityTypes/EntityDetailDrawer";

type Props = {
  env: string;
  entity?: Entity;
  entityTypes: EntityType[];
  canManage: boolean;
  onClose: () => void;
  onEntityChanged: (entity: Entity) => Promise<void>;
};

export default function EntityDrawer({ env, entity, entityTypes, canManage, onClose, onEntityChanged }: Props) {
  return (
    <EntityDetailDrawer
      env={env}
      entity={entity}
      entityType={entity ? entityTypes.find((item) => item.id === entity.entity_type_id) : undefined}
      canManage={canManage}
      onClose={onClose}
      onEntityChanged={onEntityChanged}
    />
  );
}
