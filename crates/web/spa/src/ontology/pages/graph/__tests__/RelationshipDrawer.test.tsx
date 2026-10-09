// @vitest-environment jsdom
import { ConfigProvider } from "antd";
import "../../../testSetup";
import { App as AntdApp } from "antd";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import RelationshipDrawer from "../RelationshipDrawer";

const { updateRelationship } = vi.hoisted(() => ({ updateRelationship: vi.fn().mockResolvedValue({ item: {} }) }));
vi.mock("../../../api", () => ({ api: { updateRelationship } }));

function renderDrawer(onChanged = vi.fn().mockResolvedValue(undefined)) {
  render(
    <ConfigProvider theme={{ token: { motion: false } }}><AntdApp>
      <RelationshipDrawer
        env="debug"
        relationship={{
          id: "relationship",
          env_num: 1,
          relationship_type_id: "depends",
          source_entity_id: "source",
          target_entity_id: "target",
          description: "依赖关系",
          revision: 7,
          is_deleted: false,
          is_pinned: true,
        }}
        entities={[
          { id: "source", env_num: 1, entity_type_id: "type", name: "源", description: "", revision: 1, is_deleted: false },
          { id: "target", env_num: 1, entity_type_id: "type", name: "目标", description: "", revision: 1, is_deleted: false },
        ]}
        relationshipTypes={[{ id: "depends", env_num: 1, type_key: "depends", name: "依赖", description: "", is_directory_membership: false, is_system: false, revision: 1, is_deleted: false }]}
        canManage
        onClose={vi.fn()}
        onChanged={onChanged}
      />
    </AntdApp></ConfigProvider>,
  );
  return onChanged;
}

describe("RelationshipDrawer", () => {
  it("shows endpoints, relationship type and description without pinning controls", async () => {
    renderDrawer();
    expect(await screen.findByText("源")).toBeInTheDocument();
    expect(screen.getByText("目标")).toBeInTheDocument();
    expect(screen.getByText("依赖")).toBeInTheDocument();
    expect(screen.getByText("依赖关系", { selector: ".ant-descriptions-item-content" })).toBeInTheDocument();
    expect(screen.queryByText("固定观测")).not.toBeInTheDocument();
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
  });

  it("edits the description with revision protection", async () => {
    const onChanged = renderDrawer();
    fireEvent.click(await screen.findByRole("button", { name: /编\s*辑/ }));
    const dialog = await waitFor(() => {
      const el = document.body.querySelector<HTMLElement>(".ant-modal");
      expect(el).not.toBeNull();
      return el!;
    });
    fireEvent.change(within(dialog).getByRole("textbox"), { target: { value: "新的描述" } });
    const submit = dialog.querySelector<HTMLButtonElement>("button.ant-btn-primary");
    fireEvent.click(submit!);
    await waitFor(() => expect(updateRelationship).toHaveBeenCalledWith("debug", "relationship", {
      description: "新的描述",
      is_deleted: false,
      expected_revision: 7,
    }));
    expect(onChanged).toHaveBeenCalled();
  });
});
