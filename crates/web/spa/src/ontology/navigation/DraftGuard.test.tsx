// @vitest-environment jsdom
import { ConfigProvider } from "antd";
import "../testSetup";
import { App, Button, Input } from "antd";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { expect, it, vi } from "vitest";
import { DraftGuardProvider, useDraftGuard } from "./DraftGuard";
import { ModalForm } from "../ui/ModalForm";
import { TextField } from "../ui/fields";

function Draft({ saving = false }: { saving?: boolean }) {
  const [value, setValue] = useState("");
  useDraftGuard(Boolean(value), saving);
  return <Input aria-label="正文草稿" value={value} onChange={(event) => setValue(event.target.value)} />;
}
function Navigation() {
  const [page, setPage] = useState("图谱");
  const guard = useDraftGuard();
  return <><span>{page}</span><Button onClick={() => guard.run(() => setPage("项目"))}>切换页面</Button></>;
}
it("protects registered drafts on page navigation and browser unload", async () => {
  render(<ConfigProvider theme={{ token: { motion: false } }}><App><DraftGuardProvider><Draft /><Navigation /></DraftGuardProvider></App></ConfigProvider>);
  fireEvent.change(screen.getByLabelText("正文草稿"), { target: { value: "未提交" } });
  const unload = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(unload);
  expect(unload.defaultPrevented).toBe(true);
  fireEvent.click(screen.getByText("切换页面"));
  await screen.findByText("有内容尚未保存", { selector: ".ant-modal-confirm-title" });
  fireEvent.click(screen.getByText("继续编辑"));
  expect(screen.getByLabelText("正文草稿")).toHaveValue("未提交");
  expect(screen.getByText("图谱")).toBeVisible();
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  fireEvent.click(screen.getByText("切换页面"));
  fireEvent.click(await screen.findByText("放弃并继续"));
  await screen.findByText("项目");
});

it("blocks navigation while saving and unregisters an unmounted draft", async () => {
  const view = (editing: boolean) => <ConfigProvider theme={{ token: { motion: false } }}><App><DraftGuardProvider>{editing && <Draft saving />}<Navigation /></DraftGuardProvider></App></ConfigProvider>;
  const rendered = render(view(true));
  fireEvent.click(screen.getByText("切换页面"));
  await screen.findByText("内容正在保存，请稍候");
  expect(screen.getByText("图谱")).toBeVisible();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  rendered.rerender(view(false));
  fireEvent.click(screen.getByText("切换页面"));
  await screen.findByText("项目");
});

it("preserves failed modal submissions and confirms discarding the form", async () => {
  const save = vi.fn().mockRejectedValue(new Error("版本冲突"));
  render(<ConfigProvider theme={{ token: { motion: false } }}><App><DraftGuardProvider><ModalForm title="编辑实体" trigger={<Button>打开编辑</Button>} onFinish={save}>
    <TextField name="name" label="名称" />
  </ModalForm><Navigation /></DraftGuardProvider></App></ConfigProvider>);
  fireEvent.click(screen.getByText("打开编辑"));
  fireEvent.change(await screen.findByLabelText("名称"), { target: { value: "保留输入" } });
  fireEvent.click(screen.getByRole("button", { name: "OK" }));
  await screen.findByText("版本冲突");
  expect(screen.getByLabelText("名称")).toHaveValue("保留输入");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await screen.findByText("有内容尚未保存", { selector: ".ant-modal-confirm-title" });
  fireEvent.click(screen.getByText("放弃并继续"));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "编辑实体" })).not.toBeInTheDocument());
  fireEvent.click(screen.getByText("切换页面"));
  await screen.findByText("项目");
  expect(save).toHaveBeenCalledTimes(1);
});
