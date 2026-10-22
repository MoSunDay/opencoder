import { describe, expect, it } from "vitest";
import { descendantDirectories, directoryMembers, directoryOptions, directoryTree } from "./directories";
import type { DirectoryItem, Relationship, RelationshipType } from "../../types";

const directory = (id: string, name: string, parent_id?: string | null, is_deleted = false): DirectoryItem =>
  ({ id, name, parent_id, is_deleted, description: "", revision: 1 });
const tree = [directory("root", "Root"), directory("a", "Group A", "root"), directory("b", "Group B", "root"),
  directory("aa", "Shared", "a"), directory("bb", "Shared", "b"), directory("gone", "Deleted", "a", true), directory("hidden", "Child", "gone")];
const edge = (id: string, source_entity_id: string, target_entity_id: string, relationship_type_id = "membership", is_deleted = false) =>
  ({ id, source_entity_id, target_entity_id, relationship_type_id, is_deleted } as Relationship);
const types = [{ id: "membership", is_directory_membership: true }, { id: "other", is_directory_membership: false }] as RelationshipType[];

describe("directory browsing", () => {
  it("aggregates descendants and deduplicates only active directory memberships", () => {
    const members = directoryMembers(tree, [edge("1", "one", "a"), edge("2", "two", "aa"), edge("3", "two", "a"),
      edge("4", "other", "bb"), edge("5", "wrong", "aa", "other"), edge("6", "old", "aa", "membership", true), edge("7", "hidden", "hidden")], types, "a");
    expect([...members].sort()).toEqual(["one", "two"]);
    expect([...directoryMembers(tree, [edge("1", "one", "a"), edge("2", "two", "aa")], types, "aa")]).toEqual(["two"]);
  });
  it("distinguishes equal leaf names by full path and excludes deleted options", () => {
    const options = directoryOptions(tree);
    expect(options.find(item => item.value === "aa")?.label).toBe("Root / Group A / Shared");
    expect(options.find(item => item.value === "bb")?.label).toBe("Root / Group B / Shared");
    expect(options.some(item => item.value === "gone")).toBe(false);
  });
  it("terminates malformed cycles and handles missing or removed selections", () => {
    const cycle = [directory("a", "A", "b"), directory("b", "B", "a")];
    expect([...descendantDirectories(cycle, "a")].sort()).toEqual(["a", "b"]);
    expect(directoryOptions(cycle)).toHaveLength(2);
    expect(descendantDirectories(tree, "missing").size).toBe(0);
    expect(descendantDirectories(tree, "gone").size).toBe(0);
  });
});

describe("directory tree folding", () => {
  it("nests JSON null roots instead of flattening every directory", () => {
    const nodes = directoryTree([directory("root", "Root", null), directory("a", "Group A", "root"),
      directory("b", "Group B", "root"), directory("aa", "Nested", "a")]);
    expect(nodes).toHaveLength(1);
    expect(nodes[0].children?.map(item => item.key)).toEqual(["a", "b"]);
    expect(nodes[0].children?.[0].children?.[0].key).toBe("aa");
  });
  it("keeps orphan directories reachable and terminates cycles", () => {
    const nodes = directoryTree([directory("a", "A", "missing"), directory("gone", "Gone", undefined, true),
      directory("x", "X", "y"), directory("y", "Y", "x")]);
    expect(nodes.map(item => item.key)).toEqual(["a", "x"]);
    expect(nodes[1].children?.[0].key).toBe("y");
    expect(nodes[1].children?.[0].children).toEqual([]);
    expect(directoryTree([])).toEqual([]);
  });
});
