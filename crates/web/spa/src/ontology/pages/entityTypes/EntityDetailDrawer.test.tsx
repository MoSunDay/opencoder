// @vitest-environment jsdom
import { ConfigProvider } from "antd";
import "../../testSetup";
import { App } from "antd";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../../api";
import type { Entity, EntityType } from "../../types";
import EntityDetailDrawer from "./EntityDetailDrawer";
vi.mock("../../api", () => ({ api: { attributes: vi.fn(), entity: vi.fn(), textContent: vi.fn(), setText: vi.fn(), setNfsPath: vi.fn(), setAttribute: vi.fn() } }));
beforeEach(() => vi.resetAllMocks());
it("ignores a previous entity response after the drawer selection changes", async () => {
  const type: EntityType = { id: "t", env_num: 1, type_key: "custom", name: "类型", description: "", revision: 1, is_deleted: false, is_system: false };
  const first: Entity = { id: "first", env_num: 1, entity_type_id: "t", name: "第一个", description: "旧实体详情", revision: 1, is_deleted: false };
  const second: Entity = { ...first, id: "second", name: "第二个", description: "当前实体详情" };
  const detail = (item: Entity) => ({ item, structured_attributes: [], text_attributes: [], actions: [] });
  let resolveFirst!: (value: ReturnType<typeof detail>) => void;
  vi.mocked(api.attributes).mockResolvedValue({ items: [] });
  vi.mocked(api.entity).mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; })).mockResolvedValueOnce(detail(second));
  const view = (entity: Entity) => <ConfigProvider theme={{ token: { motion: false } }}><App><EntityDetailDrawer env="debug" entity={entity} entityType={type} canManage={false} onClose={vi.fn()} onEntityChanged={vi.fn()} /></App></ConfigProvider>;
  const rendered = render(view(first));
  rendered.rerender(view(second));
  await screen.findByText("当前实体详情");
  await act(async () => { resolveFirst(detail(first)); });
  expect(screen.getByText("当前实体详情")).toBeInTheDocument();
  expect(screen.queryByText("旧实体详情")).not.toBeInTheDocument();
});

it("renders only the selected detail section", async () => {
  const type: EntityType = { id: "t", env_num: 1, type_key: "custom", name: "类型", description: "", revision: 1, is_deleted: false, is_system: false };
  const entity: Entity = { id: "one", env_num: 1, entity_type_id: "t", name: "节点", description: "节点描述", revision: 1, is_deleted: false };
  const definition = (id: string, name: string, role: "custom" | "ext" | "source", kind: "string" | "text") => ({
    id, env_num: 1, entity_type_id: "t", attribute_key: id, name, description: "", kind,
    attribute_role: role, storage_mode: kind === "text" ? "markdown" as const : "sql" as const,
    required: false, revision: 1, is_deleted: false,
  });
  const ordinary = definition("ordinary", "负责人", "custom", "string");
  const customText = definition("custom-text", "备注正文", "custom", "text");
  const extension = definition("ext", "拓展正文", "ext", "text");
  const source = definition("source", "来源正文", "source", "text");
  vi.mocked(api.attributes).mockResolvedValue({ items: [ordinary, customText, extension, source] });
  vi.mocked(api.entity).mockResolvedValue({ item: entity,
    structured_attributes: [{ attribute_definition_id: ordinary.id, value: "张三", revision: 1 }],
    text_attributes: [customText, extension, source].map((item) => ({ definition: item, current: { revision: 1, bytes: 4 } })),
    actions: [],
  });
  vi.mocked(api.textContent).mockImplementation(async (_env, _entity, id) => ({ format: "md", content: `${id} 内容`, revision: 1 }));
  render(<ConfigProvider theme={{ token: { motion: false } }}><App><EntityDetailDrawer env="debug" entity={entity} entityType={type} canManage={false}
    onClose={vi.fn()} onEntityChanged={vi.fn()} /></App></ConfigProvider>);
  await screen.findByText("节点描述");
  expect(screen.queryByText("负责人")).not.toBeInTheDocument();
  expect(screen.queryByText("拓展正文")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: /^普通属性/ }));
  expect(await screen.findByText("张三")).toBeInTheDocument();
  expect(screen.getByText("备注正文")).toBeInTheDocument();
  expect(screen.queryByText("节点描述")).not.toBeInTheDocument();
  expect(screen.queryByText("拓展正文")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "拓展信息" }));
  expect(await screen.findByText("拓展正文")).toBeInTheDocument();
  expect(screen.queryByText("备注正文")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: /^来源/ }));
  expect(await screen.findByText("来源正文")).toBeInTheDocument();
  expect(screen.queryByText("拓展正文")).not.toBeInTheDocument();
});

it("loads text on demand, previews by default, and retains an edit across tabs", async () => {
  const type: EntityType = { id: "t", env_num: 1, type_key: "custom", name: "类型", description: "", revision: 1, is_deleted: false, is_system: false };
  const entity: Entity = { id: "draft", env_num: 1, entity_type_id: "t", name: "草稿节点", description: "初始节点描述", revision: 1, is_deleted: false };
  const definition = { id: "source", env_num: 1, entity_type_id: "t", attribute_key: "source", name: "来源正文", kind: "text" as const,
    attribute_role: "source" as const, description: "", storage_mode: "markdown" as const, required: true, revision: 1, is_deleted: false };
  vi.mocked(api.attributes).mockResolvedValue({ items: [definition] });
  vi.mocked(api.entity).mockResolvedValue({ item: entity, structured_attributes: [], text_attributes: [{ definition, current: { revision: 1, bytes: 4 } }], actions: [] });
  vi.mocked(api.textContent).mockResolvedValue({ format: "md", content: "原始来源", revision: 1 });
  vi.mocked(api.textContent).mockClear();
  render(<ConfigProvider theme={{ token: { motion: false } }}><App><EntityDetailDrawer env="debug" entity={entity} entityType={type} canManage onClose={vi.fn()} onEntityChanged={vi.fn()} /></App></ConfigProvider>);
  await screen.findByText("初始节点描述", { selector: ".ant-descriptions-item-content" }); expect(api.textContent).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("tab", { name: /^来源/ })); await screen.findByText("原始来源");
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /编辑内容/ }));
  fireEvent.change(screen.getByRole("textbox", { name: "来源正文正文" }), { target: { value: "新来源" } });
  fireEvent.click(screen.getByRole("tab", { name: "基本信息" })); fireEvent.click(screen.getByRole("tab", { name: /^来源/ }));
  expect(screen.getByRole("textbox", { name: "来源正文正文" })).toHaveValue("新来源");
  fireEvent.click(screen.getByRole("button", { name: /取\s*消/ })); expect(await screen.findByText("原始来源")).toBeInTheDocument();
  vi.mocked(api.setText).mockRejectedValueOnce(new Error("版本冲突"));
  fireEvent.click(screen.getByRole("button", { name: /编辑内容/ }));
  fireEvent.change(screen.getByRole("textbox", { name: "来源正文正文" }), { target: { value: "待保存来源" } });
  fireEvent.click(screen.getByRole("button", { name: /保\s*存/ }));
  await screen.findByText("版本冲突");
  expect(screen.getByRole("textbox", { name: "来源正文正文" })).toHaveValue("待保存来源");
  expect(api.setText).toHaveBeenCalledWith("debug", "draft", "source", { format: "md", content: "待保存来源", expected_revision: 1 });
});

it("preserves structured edits across tabs, confirms close, and blocks closing during a save", async () => {
  const type: EntityType = { id: "t", env_num: 1, type_key: "custom", name: "关联系统", description: "", revision: 1, is_deleted: false, is_system: false };
  const entity: Entity = { id: "system", env_num: 1, entity_type_id: "t", name: "系统节点", description: "", revision: 1, is_deleted: false };
  const owner = { id: "owner", env_num: 1, entity_type_id: "t", attribute_key: "owner", name: "负责人", kind: "string" as const,
    attribute_role: "custom" as const, description: "", storage_mode: "sql" as const, required: false, revision: 1, is_deleted: false };
  vi.mocked(api.attributes).mockResolvedValue({ items: [owner] });
  vi.mocked(api.entity).mockResolvedValue({ item: entity, structured_attributes: [{ attribute_definition_id: "owner", value: "原负责人", revision: 1 }], text_attributes: [] });
  const onClose = vi.fn();
  render(<ConfigProvider theme={{ token: { motion: false } }}><App><EntityDetailDrawer env="debug" entity={entity} entityType={type} canManage onClose={onClose} onEntityChanged={vi.fn()} /></App></ConfigProvider>);
  await screen.findByText("关联系统");
  fireEvent.click(screen.getByRole("tab", { name: /^普通属性/ }));
  fireEvent.change(await screen.findByRole("textbox", { name: "编辑 负责人" }), { target: { value: "新负责人" } });
  fireEvent.click(screen.getByRole("tab", { name: "基本信息" }));
  fireEvent.click(screen.getByRole("tab", { name: /^普通属性/ }));
  expect(await screen.findByRole("textbox", { name: "编辑 负责人" })).toHaveValue("新负责人");
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  await screen.findByText("有内容尚未保存", { selector: ".ant-modal-confirm-title" }); expect(onClose).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "继续编辑" }));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "有内容尚未保存" })).not.toBeInTheDocument());
  let finish!: (value: object) => void;
  vi.mocked(api.setAttribute).mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
  fireEvent.click(screen.getByRole("button", { name: "提交" }));
  await waitFor(() => expect(api.setAttribute).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole("tab", { name: "基本信息" }));
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  await screen.findByText("内容正在保存，请稍候"); expect(onClose).not.toHaveBeenCalled();
  await act(async () => finish({}));
  await waitFor(() => expect(api.entity).toHaveBeenCalledTimes(2));
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
});

it("previews custom HTML in a sandbox, exposes its source, and saves the same format", async () => {
  const type: EntityType = { id: "t", env_num: 1, type_key: "custom", name: "自定义类型", description: "", revision: 1, is_deleted: false, is_system: false };
  const entity: Entity = { id: "html-node", env_num: 1, entity_type_id: "t", name: "HTML 节点", description: "", revision: 1, is_deleted: false };
  const definition = { id: "html", env_num: 1, entity_type_id: "t", attribute_key: "html", name: "备注正文", kind: "text" as const,
    attribute_role: "custom" as const, description: "", storage_mode: "markdown" as const, required: false, revision: 1, is_deleted: false };
  const content = "<div><p>已保存的 HTML 正文</p></div>";
  vi.mocked(api.attributes).mockResolvedValue({ items: [definition] });
  vi.mocked(api.entity).mockResolvedValue({ item: entity, structured_attributes: [], text_attributes: [{ definition, current: { revision: 1 } }] });
  vi.mocked(api.textContent).mockResolvedValue({ format: "html", content, revision: 1 });
  vi.mocked(api.setText).mockResolvedValue({ revision: 2 });
  render(<ConfigProvider theme={{ token: { motion: false } }}><App><EntityDetailDrawer env="debug" entity={entity} entityType={type} canManage onClose={vi.fn()} onEntityChanged={vi.fn()} /></App></ConfigProvider>);
  await screen.findByText("自定义类型"); fireEvent.click(screen.getByRole("tab", { name: /^普通属性/ }));
  const preview = await screen.findByTitle("备注正文预览");
  expect(preview).toHaveAttribute("sandbox", "");
  expect(preview.getAttribute("srcdoc")).toContain(content);
  fireEvent.click(screen.getByText("源码")); expect(screen.getByText(content)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /编辑内容/ }));
  fireEvent.change(screen.getByRole("textbox", { name: "备注正文正文" }), { target: { value: "<p>更新正文</p>" } });
  fireEvent.click(screen.getByRole("button", { name: /保\s*存/ }));
  await waitFor(() => expect(api.setText).toHaveBeenCalledWith("debug", "html-node", "html", { format: "html", content: "<p>更新正文</p>", expected_revision: 1 }));
  expect(await screen.findByTitle("备注正文预览")).toHaveAttribute("srcdoc", expect.stringContaining("<p>更新正文</p>"));
});
