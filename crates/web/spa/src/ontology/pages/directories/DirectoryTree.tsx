import { MoreOutlined } from "@ant-design/icons";
import { App, Button, Dropdown, Input, Modal, Select, Tree } from "antd";
import type { InputRef, MenuProps, TreeDataNode, TreeProps } from "antd";
import { useEffect, useMemo, useRef, useState, type ReactNode, type SyntheticEvent } from "react";
import { api } from "../../api";
import { useDraftGuard } from "../../navigation/DraftGuard";
import type { Directory } from "../../types";
import { buildDirectoryTree, descendantIds, siblingNameExists } from "./directoryModel";
import "./directories.css";

const DRAFT_KEY = "__draft__";
type DirectoryDraft = { action: "create" | "rename"; directoryId: string; parentId: string };

/** Menu entries per node; the synthesized root may only gain children. */
export const directoryMenuItems = (isRoot: boolean): MenuProps["items"] => isRoot
  ? [{ key: "create", label: "新增子目录" }]
  : [{ key: "create", label: "新增子目录" }, { type: "divider" as const }, { key: "rename", label: "重命名" }, { key: "move", label: "移动到…" }, { key: "delete", label: "删除目录", danger: true }];

/** VSCode-style inline draft row: autofocus, Enter commits, Esc/blur cancels. */
function DraftInput({ label, initial, value, invalid, onChange, onSubmit, onCancel }: {
  label: string; initial?: string; value: string; invalid: boolean;
  onChange: (next: string) => void; onSubmit: () => void; onCancel: () => void;
}) {
  const ref = useRef<InputRef>(null);
  useEffect(() => { ref.current?.focus({ cursor: initial ? "all" : "end" }); }, [initial]);
  const stop = (event: SyntheticEvent) => event.stopPropagation();
  return <Input ref={ref} size="small" className="dir-tree-input" aria-label={label} placeholder="目录名称" value={value}
    status={invalid ? "error" : undefined} onClick={stop}
    onChange={(event) => onChange(event.target.value)}
    onKeyDown={(event) => { event.stopPropagation(); if (event.key === "Enter") onSubmit(); else if (event.key === "Escape") onCancel(); }}
    onBlur={onCancel} />;
}

/** One tree row: the directory name plus the hover `···` menu (same menu as right-click). */
function DirectoryRow({ directory, isRoot, canManage, onCommand }: {
  directory: Directory; isRoot: boolean; canManage: boolean; onCommand: (key: string, target: Directory) => void;
}) {
  const menu = { items: directoryMenuItems(isRoot), onClick: ({ key }: { key: string }) => onCommand(key, directory) };
  return <span className="dir-tree-row" data-directory-id={directory.id}>
    <span className="dir-tree-name" title={directory.description || directory.name}>{directory.name}</span>
    {canManage ? <span className="dir-tree-more-holder" onClick={(event) => event.stopPropagation()}>
      <Dropdown trigger={["click"]} menu={menu}>
        <Button type="text" size="small" className="dir-tree-more" aria-label={`目录操作 ${directory.name}`} icon={<MoreOutlined />} />
      </Dropdown>
    </span> : null}
  </span>;
}

type Props = {
  directories: Directory[]; rootId: string; selectedId?: string; canManage: boolean; env: string;
  onSelect: (id: string) => void; reload: () => Promise<void>; createRootSignal?: number;
};

export default function DirectoryTree({ directories, rootId, selectedId, canManage, env, onSelect, reload, createRootSignal }: Props) {
  const { message, modal } = App.useApp();
  const [draft, setDraft] = useState<DirectoryDraft>();
  const [draftValue, setDraftValue] = useState("");
  const [expanded, setExpanded] = useState<string[]>(() => [rootId]);
  const [menuTargetId, setMenuTargetId] = useState(rootId);
  const [moving, setMoving] = useState<Directory>();
  const [moveParentId, setMoveParentId] = useState<string>();
  const [saving, setSaving] = useState(false);
  useDraftGuard(Boolean(draft), saving);
  const tree = useMemo(() => buildDirectoryTree(directories, rootId), [directories, rootId]);
  const directoryById = useMemo(() => new Map(directories.map((item) => [item.id, item])), [directories]);
  const rootName = directoryById.get(rootId)?.name ?? "根目录";
  const rootDirectory: Directory = directoryById.get(rootId) ?? { id: rootId, name: rootName, description: "", parent_id: null, revision: 0, is_deleted: false };
  const parentOf = (id: string) => directoryById.get(id)?.parent_id ?? rootId;
  const draftError = draft && draftValue.trim() && siblingNameExists(directories, draft.parentId, draftValue, draft.action === "rename" ? draft.directoryId : undefined) ? "同名目录已存在" : "";
  const startCreate = (parentId: string) => {
    setDraft({ action: "create", directoryId: parentId, parentId });
    setDraftValue("");
    setExpanded((prev) => (prev.includes(parentId) ? prev : [...prev, parentId]));
  };
  useEffect(() => { if (createRootSignal) startCreate(rootId); }, [createRootSignal, rootId]);
  useEffect(() => {
    if (!selectedId) return;
    const chain: string[] = [];
    let current = directoryById.get(selectedId)?.parent_id;
    while (current) { chain.push(current); current = directoryById.get(current)?.parent_id; }
    setExpanded((prev) => (chain.every((id) => prev.includes(id)) ? prev : [...new Set([...prev, ...chain])]));
  }, [selectedId, directoryById]);
  const cancelDraft = () => { setDraft(undefined); setDraftValue(""); };
  const submitDraft = async () => {
    if (!draft || !draftValue.trim() || draftError) return;
    const name = draftValue.trim();
    setSaving(true);
    try {
      if (draft.action === "create") {
        await api.createDirectory(env, { name, parent_id: draft.parentId });
        message.success("目录已创建");
      } else {
        const current = directoryById.get(draft.directoryId);
        await api.updateDirectory(env, draft.directoryId, { name, description: current?.description ?? "", is_deleted: false, expected_revision: current?.revision ?? 0 });
        message.success("目录已更新");
      }
      cancelDraft();
      await reload();
    } catch (reason) { message.error(reason instanceof Error ? reason.message : "操作失败"); } finally { setSaving(false); }
  };
  const confirmRemove = (target: Directory) => {
    modal.confirm({ title: "确认删除目录？", content: "目录内还有成员或子目录时会删除失败，请先移出。", okText: "删除", cancelText: "取消", okButtonProps: { danger: true },
      onOk: async () => {
        setSaving(true);
        try {
          await api.updateDirectory(env, target.id, { name: target.name, description: target.description, is_deleted: true, expected_revision: target.revision });
          message.success("目录已删除");
          await reload();
        } catch (reason) { message.error(reason instanceof Error ? reason.message : "操作失败"); throw reason; } finally { setSaving(false); }
      } });
  };
  const onCommand = (key: string, target: Directory) => {
    if (key === "create") startCreate(target.id);
    else if (key === "rename") { setDraft({ action: "rename", directoryId: target.id, parentId: target.parent_id ?? rootId }); setDraftValue(target.name); }
    else if (key === "move") { setMoving(target); setMoveParentId(target.parent_id ?? rootId); }
    else if (key === "delete") confirmRemove(target);
  };
  const move = async (id: string, parentId: string) => {
    setSaving(true);
    try { await api.moveDirectory(env, id, parentId); message.success("目录已移动"); await reload(); return true; }
    catch (reason) { message.error(reason instanceof Error ? reason.message : "移动失败"); return false; }
    finally { setSaving(false); }
  };
  const moveOptions = useMemo(() => {
    if (!moving) return [];
    const blocked = descendantIds(directories, moving.id);
    const options = directories.filter((item) => !item.is_deleted && !blocked.has(item.id))
      .map((item) => ({ value: item.id, label: item.id === rootId ? rootName : item.name }))
      .sort((left, right) => left.label.localeCompare(right.label));
    return options.some((option) => option.value === rootId) ? [{ value: rootId, label: rootName }, ...options.filter((option) => option.value !== rootId)] : options;
  }, [moving, directories, rootId, rootName]);
  const onDrop: TreeProps["onDrop"] = (info) => {
    const dragId = String(info.dragNode.key);
    const dropId = String(info.node.key);
    if (dragId === rootId) { message.error("根目录不能移动"); return; }
    const blocked = descendantIds(directories, dragId);
    const parentId = info.dropToGap ? parentOf(dropId) : dropId;
    if (parentId === dragId || blocked.has(parentId)) { message.error("不能移动到自身或其子目录下"); return; }
    if (parentOf(dragId) === parentId) return;
    void move(dragId, parentId);
  };
  const draftInput = draft
    ? <DraftInput label={draft.action === "create" ? "新增目录名称" : "重命名目录名称"} initial={draft.action === "rename" ? directoryById.get(draft.directoryId)?.name : undefined}
      value={draftValue} invalid={Boolean(draftError)} onChange={setDraftValue} onSubmit={() => void submitDraft()} onCancel={cancelDraft} />
    : null;
  const treeData = useMemo<TreeDataNode[]>(() => {
    if (!draft || draft.action !== "create") return tree as TreeDataNode[];
    const draftNode: TreeDataNode = { key: DRAFT_KEY, title: "", selectable: false, children: [] };
    const insert = (nodes: TreeDataNode[]): TreeDataNode[] => nodes.map((node) => node.key === draft.parentId
      ? { ...node, children: [...(node.children ?? []), draftNode] }
      : { ...node, children: insert(node.children ?? []) });
    return insert(tree as TreeDataNode[]);
  }, [tree, draft]);
  const renderTitle = (node: TreeDataNode): ReactNode => {
    if (node.key === DRAFT_KEY) return draftInput;
    if (draft?.action === "rename" && node.key === draft.directoryId) return draftInput;
    const directory = directoryById.get(String(node.key)) ?? rootDirectory;
    return <DirectoryRow directory={directory} isRoot={String(node.key) === rootId} canManage={canManage} onCommand={onCommand} />;
  };
  const content = <div className="dir-tree-viewport" onContextMenu={canManage ? (event) => {
    const holder = (event.target as HTMLElement).closest(".ant-tree-treenode");
    setMenuTargetId(holder?.querySelector("[data-directory-id]")?.getAttribute("data-directory-id") ?? rootId);
  } : undefined}>
    <Tree blockNode treeData={treeData} titleRender={renderTitle} draggable={canManage && { icon: false }} onDrop={onDrop}
      expandedKeys={expanded} onExpand={(keys) => setExpanded(keys.map(String))}
      selectedKeys={selectedId ? [selectedId] : []}
      onSelect={(keys) => { if (keys.length > 0) onSelect(String(keys[0])); }} />
    {draft && draftError ? <div className="dir-tree-error" role="alert">{draftError}</div> : null}
  </div>;
  const menuTarget = directoryById.get(menuTargetId) ?? rootDirectory;
  return <aside className="dir-tree" aria-label="目录树">
    {canManage
      ? <Dropdown trigger={["contextMenu"]} menu={{ items: directoryMenuItems(menuTarget.id === rootId), onClick: ({ key }) => onCommand(key, menuTarget) }}>{content}</Dropdown>
      : content}
    <Modal title={`移动目录：${moving?.name ?? ""}`} open={Boolean(moving)} okText="移动" cancelText="取消" confirmLoading={saving} destroyOnHidden
      onOk={async () => { if (moving && moveParentId && await move(moving.id, moveParentId)) { setMoving(undefined); setMoveParentId(undefined); } }}
      onCancel={() => { setMoving(undefined); setMoveParentId(undefined); }}>
      <Select aria-label="目标目录" className="dir-move-select" showSearch optionFilterProp="label" placeholder="选择目标目录"
        value={moveParentId} onChange={setMoveParentId} options={moveOptions} />
    </Modal>
  </aside>;
}
