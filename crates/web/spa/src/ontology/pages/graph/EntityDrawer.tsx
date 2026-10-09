import type { Entity, EntityType } from "../../types";
import EntityDetailDrawer from "../entityTypes/EntityDetailDrawer";
import type { ComponentProps } from "react";

type Props = {
  env: string;
  entity?: Entity;
  entityTypes: EntityType[];
  canManage: boolean;
  onClose: () => void;
  onEntityChanged: (entity: Entity) => Promise<void>;
};

export default function EntityDrawer({ env, entity, entityTypes, canManage, onClose, onEntityChanged, ...detail }: Props & Pick<ComponentProps<typeof EntityDetailDrawer>, "relations" | "onBack" | "onObserve" | "view" | "onView">) {
  return (
    <EntityDetailDrawer
      {...detail}
      env={env}
      entity={entity}
      entityType={entity ? entityTypes.find((item) => item.id === entity.entity_type_id) : undefined}
      canManage={canManage}
      onClose={onClose}
      onEntityChanged={onEntityChanged}
    />
  );
}
