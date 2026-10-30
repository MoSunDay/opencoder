import { ReloadOutlined } from "@ant-design/icons";
import { Button } from "antd";
import { useEffect, useMemo, useState } from "react";
import type { Directory, Entity, EntityType, Relationship, RelationshipType } from "../../types";
import DirectoryTree from "./DirectoryTree";
import MembersPanel from "./MembersPanel";
import { directoryMembers, membershipType } from "./directoryModel";
import "./directories.css";

type Props = {
  env: string; canManage: boolean; directories: Directory[]; rootId: string; entities: Entity[];
  relationships: Relationship[]; relationshipTypes: RelationshipType[]; entityTypes?: EntityType[];
  reload: () => Promise<void>; focusDirectoryId?: string;
};

const fallbackRoot = (rootId: string): Directory => ({ id: rootId, name: "根目录", description: "", parent_id: null, revision: 0, is_deleted: false });

export default function DirectoryPanel({ env, canManage, directories, rootId, entities, relationships, relationshipTypes, entityTypes, reload, focusDirectoryId }: Props) {
  const [selectedId, setSelectedId] = useState(focusDirectoryId ?? rootId);
  const [createRootSignal, setCreateRootSignal] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  useEffect(() => { if (focusDirectoryId) setSelectedId(focusDirectoryId); }, [focusDirectoryId]);
  const membershipTypeId = membershipType(relationshipTypes)?.id;
  const directoryById = useMemo(() => new Map(directories.map((item) => [item.id, item])), [directories]);
  const selected = directoryById.get(selectedId) ?? fallbackRoot(rootId);
  const memberships = useMemo(() => directoryMembers(relationships, membershipTypeId, selected.id), [relationships, membershipTypeId, selected.id]);
  const refresh = async () => { setRefreshing(true); try { await reload(); } finally { setRefreshing(false); } };
  return <div className="dir-panel">
    <div className="dir-toolbar">
      <Button icon={<ReloadOutlined />} loading={refreshing} onClick={() => void refresh()}>刷新</Button>
      {canManage ? <Button type="primary" onClick={() => setCreateRootSignal((value) => value + 1)}>新增根目录</Button> : null}
    </div>
    <div className="dir-layout">
      <DirectoryTree directories={directories} rootId={rootId} selectedId={selected.id} canManage={canManage} env={env}
        onSelect={setSelectedId} reload={reload} createRootSignal={createRootSignal} />
      <MembersPanel env={env} canManage={canManage} directory={selected} isRoot={selected.id === rootId}
        entities={entities} entityTypes={entityTypes} memberships={memberships} membershipTypeId={membershipTypeId} reload={reload} />
    </div>
  </div>;
}
