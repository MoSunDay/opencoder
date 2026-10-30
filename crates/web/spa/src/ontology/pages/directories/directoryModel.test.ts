import { describe, expect, it } from "vitest";
import type { Directory, Relationship, RelationshipType } from "../../types";
import { buildDirectoryTree, descendantIds, directoryMembers, entityDirectories, membershipType, siblingNameExists } from "./directoryModel";

const dir = (id: string, name: string, parentId: string | null, extra: Partial<Directory> = {}): Directory =>
  ({ id, name, description: "", parent_id: parentId, revision: 1, is_deleted: false, ...extra });
const belongs = (id: string, source: string, target: string, extra: Partial<Relationship> = {}): Relationship =>
  ({ id, env_num: 1, relationship_type_id: "belongs", source_entity_id: source, target_entity_id: target, description: "", revision: 1, is_deleted: false, is_pinned: false, ...extra });

describe("directoryModel", () => {
  it("builds one root node with children sorted by name", () => {
    const items = [
      dir("root", "根目录", null),
      dir("b", "服务", "root"),
      dir("a", "平台", "root"),
      dir("a-1", "子模块", "a"),
      dir("gone", "已删除目录", "root", { is_deleted: true }),
    ];
    const tree = buildDirectoryTree(items, "root");
    expect(tree).toHaveLength(1);
    expect(tree[0]).toMatchObject({ key: "root", title: "根目录" });
    expect(tree[0].children.map((node) => node.key)).toEqual(["a", "b"]);
    expect(tree[0].children[0].children.map((node) => node.title)).toEqual(["子模块"]);
  });

  it("reattaches orphaned directories to the root and synthesizes a missing root name", () => {
    const tree = buildDirectoryTree([dir("orphan", "孤儿", "missing"), dir("kept", "保留", "orphan")], "root");
    expect(tree[0].title).toBe("根目录");
    expect(tree[0].children.map((node) => node.key)).toEqual(["orphan"]);
    expect(tree[0].children[0].children.map((node) => node.key)).toEqual(["kept"]);
  });

  it("detects duplicate sibling names while ignoring the renamed directory itself", () => {
    const items = [dir("root", "根目录", null), dir("a", "平台", "root"), dir("b", "服务", "root"), dir("b-1", "平台", "b")];
    expect(siblingNameExists(items, "root", "平台")).toBe(true);
    expect(siblingNameExists(items, "root", " 平台 ")).toBe(true);
    expect(siblingNameExists(items, "root", "平台", "a")).toBe(false);
    expect(siblingNameExists(items, "b", "平台")).toBe(true);
    expect(siblingNameExists(items, "root", "其他")).toBe(false);
    expect(siblingNameExists(items, "root", "  ")).toBe(false);
  });

  it("collects a directory plus all descendants for move validation", () => {
    const items = [dir("root", "根目录", null), dir("a", "平台", "root"), dir("a-1", "模块", "a"), dir("a-1-1", "子模块", "a-1"), dir("b", "服务", "root")];
    expect([...descendantIds(items, "a")].sort()).toEqual(["a", "a-1", "a-1-1"]);
    expect([...descendantIds(items, "b")]).toEqual(["b"]);
    expect(descendantIds(items, "a-1-1").has("a")).toBe(false);
  });

  it("finds the active membership relationship type", () => {
    const type = (id: string, membership: boolean, deleted = false): RelationshipType =>
      ({ id, env_num: 1, type_key: id, name: id, description: "", is_directory_membership: membership, is_system: true, revision: 1, is_deleted: deleted });
    expect(membershipType([type("plain", false), type("belongs", true)])?.id).toBe("belongs");
    expect(membershipType([type("belongs", true, true), type("plain", false)])).toBeUndefined();
    expect(membershipType([])).toBeUndefined();
  });

  it("lists direct members of a directory and directories of an entity from active memberships", () => {
    const relations = [
      belongs("r1", "entity-1", "dir-a"),
      belongs("r2", "entity-2", "dir-a"),
      belongs("r3", "entity-1", "dir-b"),
      belongs("r4", "entity-3", "dir-a", { is_deleted: true }),
      belongs("r5", "entity-1", "dir-c", { relationship_type_id: "other" }),
    ];
    expect(directoryMembers(relations, "belongs", "dir-a").map((relation) => relation.id)).toEqual(["r1", "r2"]);
    expect(directoryMembers(relations, undefined, "dir-a")).toEqual([]);
    expect(entityDirectories(relations, "belongs", "entity-1")).toEqual(["dir-a", "dir-b"]);
    expect(entityDirectories(relations, undefined, "entity-1")).toEqual([]);
  });
});
