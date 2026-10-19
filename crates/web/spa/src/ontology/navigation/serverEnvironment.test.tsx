// @vitest-environment jsdom
import "../testSetup";
import { App, ConfigProvider } from "antd";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NAV_CATEGORIES } from "../../nav.js";
import { setState } from "../../store.js";
import { OntologyTypesPanel } from "../panels";

const { request } = vi.hoisted(() => ({ request: vi.fn<(method: string, path: string, body?: unknown) => Promise<unknown>>() }));
vi.mock("../../api.js", () => ({ apiJson: request }));
const type = { id: "service", env_num: 1, type_key: "service", name: "当前 Server 的服务",
  description: "", is_system: false, revision: 1, is_deleted: false };

function renderPanel() {
  return render(<ConfigProvider theme={{ token: { motion: false } }}><App><OntologyTypesPanel /></App></ConfigProvider>);
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.setItem("oc_ontology_env", JSON.stringify("another-server"));
  setState({ identity: { name: "editor", role: "editor" } });
  request.mockImplementation(async (method) => method === "GET" ? { items: [type] } : { item: type });
});
afterEach(() => { cleanup(); localStorage.clear(); setState({ identity: null }); });

it("reads and creates in the Server namespace despite a stale browser environment selection", async () => {
  renderPanel();
  await screen.findByText(type.name);
  expect(request).toHaveBeenCalledWith("GET", "/api/ontology/envs/debug/entity-types?include_deleted=true", undefined);
  expect(screen.queryByRole("combobox", { name: "Ontology 环境" })).not.toBeInTheDocument();
  expect(NAV_CATEGORIES.find((category: { key: string }) => category.key === "ontology").items.map((item: { page: string }) => item.page))
    .toEqual(["ontologyGraph", "ontologyEntities", "ontologyTypes", "ontologyRelationships"]);

  fireEvent.click(screen.getByRole("button", { name: /新\s*增/ }));
  fireEvent.change(await screen.findByLabelText("类型 Key"), { target: { value: "new_type" } });
  fireEvent.change(screen.getByLabelText("名称", { exact: true }), { target: { value: "新类型" } });
  fireEvent.click(screen.getByRole("button", { name: "OK" }));
  await waitFor(() => expect(request).toHaveBeenCalledWith("POST", "/api/ontology/envs/debug/entity-types",
    { key: "new_type", name: "新类型" }));
  expect(request.mock.calls.every(([, path]) => String(path).startsWith("/api/ontology/envs/debug/"))).toBe(true);
});

it("keeps read failures visible and retries directly against the Server namespace", async () => {
  request.mockRejectedValueOnce(new Error("Server 暂时不可用"));
  renderPanel();
  await screen.findByText("Server 暂时不可用");
  fireEvent.click(screen.getByRole("button", { name: /重\s*试/ }));
  await screen.findByText(type.name);
  expect(request).toHaveBeenCalledTimes(2);
  expect(request.mock.calls.every(([, path]) => path === "/api/ontology/envs/debug/entity-types?include_deleted=true")).toBe(true);
});

it("preserves viewer permissions without an environment selector", async () => {
  setState({ identity: { name: "viewer", role: "viewer" } });
  renderPanel();
  await screen.findByText(type.name);
  expect(screen.queryByRole("button", { name: /新\s*增/ })).not.toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "Ontology 环境" })).not.toBeInTheDocument();
});
