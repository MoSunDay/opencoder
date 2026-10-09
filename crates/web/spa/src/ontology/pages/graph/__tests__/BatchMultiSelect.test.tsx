// @vitest-environment jsdom
import "../../../testSetup";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import BatchMultiSelect from "../BatchMultiSelect";
function Example({ onCommit }: { onCommit: (value: string[]) => void }) {
  const [value, setValue] = useState<string[]>([]);
  return <BatchMultiSelect label="测试多选" placeholder="请选择" value={value}
    onChange={(next) => { setValue(next); onCommit(next); }} options={[{ value: "a", label: "类型 A" }, { value: "b", label: "类型 B" }]} />;
}
async function choose(name: string) {
  const option = await waitFor(() => {
    const found = document.querySelector(`.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option[title="${name}"]`);
    expect(found).not.toBeNull(); return found as HTMLElement;
  });
  fireEvent.click(option);
}
describe("BatchMultiSelect", () => {
  it("commits several selections once on Done and keeps the field to one line", async () => {
    const commit = vi.fn(); render(<Example onCommit={commit} />);
    const select = screen.getByRole("combobox", { name: "测试多选" }); fireEvent.mouseDown(select);
    await choose("类型 A"); await choose("类型 B");
    expect(commit).not.toHaveBeenCalled();
    expect(select.closest(".ant-select")).toHaveTextContent("类型 A");
    expect(select.closest(".ant-select")).toHaveTextContent("+1");
    fireEvent.click(screen.getByRole("button", { name: /完\s*成/ }));
    await waitFor(() => expect(commit).toHaveBeenCalledTimes(1));
    expect(commit).toHaveBeenCalledWith(["a", "b"]);
  });
  it("commits on Escape and does not submit an unchanged selection again", async () => {
    const commit = vi.fn(); render(<Example onCommit={commit} />);
    const select = screen.getByRole("combobox", { name: "测试多选" }); fireEvent.mouseDown(select);
    await choose("类型 A"); fireEvent.keyDown(select, { key: "Escape", keyCode: 27, which: 27 });
    await waitFor(() => expect(commit).toHaveBeenCalledTimes(1));
    expect(commit).toHaveBeenCalledWith(["a"]);
    fireEvent.mouseDown(select); fireEvent.click(screen.getByRole("button", { name: /完\s*成/ }));
    expect(commit).toHaveBeenCalledTimes(1);
  });
});
