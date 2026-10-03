// @vitest-environment jsdom
import "../../../test/setup-dom.js";
import { Button } from "antd";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ModalForm } from "../ModalForm";
import { TextField } from "../fields";

afterEach(cleanup);

it("labels target their own field when other retained modals use the same field name", async () => {
  const save = vi.fn().mockResolvedValue(true);
  render(<>
    <ModalForm title="新增类型" trigger={<Button>创建类型</Button>} onFinish={vi.fn()}>
      <TextField name="key" label="类型 Key" />
    </ModalForm>
    <ModalForm title="新增属性" trigger={<Button>创建属性</Button>} onFinish={save}>
      <TextField name="key" label="属性 Key" rules={[{ required: true }]} />
    </ModalForm>
  </>);
  fireEvent.click(screen.getByRole("button", { name: "创建属性" }));
  const dialog = screen.getByRole("dialog");
  const field = within(dialog).getByLabelText("属性 Key");
  expect(field.closest(".ant-modal")).toBe(dialog);
  expect(document.querySelectorAll("input[id]")).toHaveLength(2);
  expect(new Set([...document.querySelectorAll("input[id]")].map((element) => element.id)).size).toBe(2);
  fireEvent.change(field, { target: { value: "owner" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "OK" }));
  await waitFor(() => expect(save).toHaveBeenCalledWith({ key: "owner" }));
});

it("keeps the entered value after an unsuccessful save so the user can retry", async () => {
  const save = vi.fn().mockRejectedValueOnce(new Error("保存失败")).mockResolvedValue(true);
  render(<ModalForm title="新增类型" trigger={<Button>创建类型</Button>} onFinish={save}>
    <TextField name="name" label="名称" />
  </ModalForm>);
  fireEvent.click(screen.getByRole("button", { name: "创建类型" }));
  const dialog = screen.getByRole("dialog");
  const field = within(dialog).getByLabelText("名称") as HTMLInputElement;
  fireEvent.change(field, { target: { value: "服务" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "OK" }));
  await within(dialog).findByText("保存失败");
  expect(field.value).toBe("服务");
  fireEvent.click(within(dialog).getByRole("button", { name: "OK" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(save).toHaveBeenLastCalledWith({ name: "服务" });
});
