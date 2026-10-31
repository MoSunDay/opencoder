// @vitest-environment jsdom
import "../testSetup";
import { App, ConfigProvider } from "antd";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../api";
import { EnvContext } from "../env";
import type { Entity, EntityType, FreshnessPolicy, FreshnessReview, FreshnessRun } from "../types";
import FreshnessPage from "./FreshnessPage";

vi.mock("../api", () => ({ api: {
  freshnessPolicies: vi.fn(), entityTypes: vi.fn(), triggerFreshnessRun: vi.fn(),
  freshnessRuns: vi.fn(), freshnessRun: vi.fn(), freshnessReviews: vi.fn(), createFreshnessReview: vi.fn(), entities: vi.fn(),
} }));

const type: EntityType = { id: "type", env_num: 1, type_key: "type", name: "能力", description: "", is_system: false, revision: 1, is_deleted: false };
const policy: FreshnessPolicy = {
  id: "policy-1", env_num: 1, entity_type_id: "type", name: "能力刷新策略", description: "",
  executor: { kind: "agent", target: "refresher", prompt_template: "刷新 {{entity.name}}" }, concurrency: 2,
  writeback_attribute_key: null, revision: 1, is_deleted: false,
};
const run: FreshnessRun = {
  id: "run-1", env_num: 1, policy_id: "policy-1", status: "succeeded",
  items: [{ entity_id: "entity-1", entity_name: "待评审实体", execution_id: "exec-1", status: "succeeded", candidate: "## 候选内容", error: null }],
  stats: { total: 1, pending: 0, dispatched: 0, succeeded: 1, failed: 0 },
  revision: 1, is_deleted: false, created_at: "2026-10-30T08:00:00Z", updated_at: "2026-10-30T08:05:00Z",
};
const reviewedEntity: Entity = { id: "entity-2", env_num: 1, entity_type_id: "type", name: "已评审实体", description: "", revision: 2, is_deleted: false };
const review: FreshnessReview = {
  id: "review-1", env_num: 1, run_id: "run-1", policy_id: "policy-1", entity_id: "entity-2",
  from_level: "draft", to_level: "stable", verdict: "approve", comment: "", actor: "editor", created_at: "2026-10-30T09:00:00Z",
};

beforeEach(() => {
  vi.mocked(api.freshnessPolicies).mockResolvedValue({ items: [policy] });
  vi.mocked(api.entityTypes).mockResolvedValue({ items: [type] });
  vi.mocked(api.freshnessRuns).mockResolvedValue({ items: [run] });
  vi.mocked(api.freshnessRun).mockResolvedValue({ item: run });
  vi.mocked(api.freshnessReviews).mockResolvedValue({ items: [review] });
  vi.mocked(api.entities).mockResolvedValue({ items: [
    { ...reviewedEntity, data_level: "stable" },
    { id: "entity-1", env_num: 1, entity_type_id: "type", name: "待评审实体", description: "", data_level: "stable", revision: 1, is_deleted: false },
  ] });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.resetAllMocks(); });

const open = () => render(<ConfigProvider theme={{ token: { motion: false } }}><App>
  <EnvContext.Provider value={{ env: "debug", canManage: true }}><FreshnessPage /></EnvContext.Provider>
</App></ConfigProvider>);

it("renders the three tabs and the policy rows", async () => {
  open();
  expect(screen.getByRole("tab", { name: "策略" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tab", { name: "运行" })).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "评审" })).toBeInTheDocument();
  expect(await screen.findByText("能力刷新策略")).toBeInTheDocument();
  expect(screen.getByText("refresher")).toBeInTheDocument();
  expect(screen.getAllByText("能力").length).toBeGreaterThan(0);
  expect(screen.getByRole("button", { name: /新\s*增/ })).toBeInTheDocument();
});

it("shows run status and stats in the runs tab", async () => {
  open();
  await screen.findByText("能力刷新策略");
  fireEvent.click(screen.getByRole("tab", { name: "运行" }));
  expect(await screen.findByText("成功 1 / 失败 0 / 共 1")).toBeInTheDocument();
  expect(screen.getByText("run-1")).toBeInTheDocument();
  expect(screen.getByText("policy-1")).toBeInTheDocument();
});

it("queues review candidates with level badges and the review history", async () => {
  open();
  await screen.findByText("能力刷新策略");
  fireEvent.click(screen.getByRole("tab", { name: "评审" }));
  expect(await screen.findByText("待评审实体")).toBeInTheDocument();
  expect(screen.getAllByText("候选摘要").length).toBeGreaterThan(0);
  expect(screen.getAllByText("稳定").length).toBeGreaterThanOrEqual(2);
  expect(screen.getByText("草稿")).toBeInTheDocument();
  expect(screen.getByText("待评审")).toBeInTheDocument();
  expect(screen.getAllByText(/通\s*过/).some((element) => element.closest("button"))).toBe(true);
  expect(screen.getByText(/驳\s*回/).closest("button")).toBeInTheDocument();
});
