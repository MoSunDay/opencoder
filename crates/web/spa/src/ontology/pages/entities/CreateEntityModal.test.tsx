// @vitest-environment jsdom
import { ConfigProvider } from "antd";
import "../../testSetup";
import { App } from "antd";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../../api";
import type { AttributeDefinition, EntityType } from "../../types";
import CreateEntityModal from "./CreateEntityModal";
vi.mock("../../api", () => ({ api: { attributes: vi.fn(), createEntity: vi.fn() } }));
const type: EntityType = { id: "t", env_num: 1, type_key: "example", name: "示例类型", description: "", revision: 1, is_deleted: false, is_system: false };
const definition = (id: string, role: "source" | "ext" | "custom", name: string): AttributeDefinition => ({ id, env_num: 1, entity_type_id: "t", attribute_key: role, attribute_role: role, storage_mode: "markdown", name, description: "", kind: "text", required: true, revision: 1, is_deleted: false });
async function open() {
  render(<ConfigProvider theme={{ token: { motion: false } }}><App><CreateEntityModal env="debug" types={[type]} onCreated={vi.fn()} /></App></ConfigProvider>);
  fireEvent.click(screen.getByRole("button", { name: /新增实体/ }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.mouseDown(within(dialog).getByRole("combobox"));
  fireEvent.click(await screen.findByText("示例类型"));
  await screen.findByLabelText("负责人");
  return dialog;
}
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.attributes).mockResolvedValue({ items: [definition("1", "source", "来源"), { ...definition("2", "ext", "扩展"), storage_mode: "nfs_path" }, { ...definition("9007199254740993", "custom", "负责人"), kind: "string", storage_mode: "sql" }] });
});
describe("实体创建合同", () => {
  it("loads required fields, distinguishes NFS paths and retains the request ID across a retry", async () => {
    const dialog = await open();
    expect(screen.getByLabelText("扩展文件 NFS path")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "实体" } });
    fireEvent.change(screen.getByLabelText("来源"), { target: { value: "人工录入" } });
    fireEvent.change(screen.getByLabelText("扩展文件 NFS path"), { target: { value: "debug/test.md" } });
    fireEvent.change(screen.getByLabelText("负责人"), { target: { value: "owner" } });
    vi.mocked(api.createEntity).mockRejectedValue(new Error("连接中断"));
    const submit = within(dialog).getByRole("button", { name: "OK" });
    fireEvent.click(submit);
    await waitFor(() => expect(api.createEntity).toHaveBeenCalledTimes(1));
    const payload = vi.mocked(api.createEntity).mock.calls[0][1];
    expect(payload).toMatchObject({ attributes: { "9007199254740993": "owner" }, ext: "debug/test.md" });
    await screen.findByText("连接中断");
    fireEvent.click(submit);
    await waitFor(() => expect(api.createEntity).toHaveBeenCalledTimes(2));
    expect(vi.mocked(api.createEntity).mock.calls[1][1]).toEqual(payload);
  });
});
