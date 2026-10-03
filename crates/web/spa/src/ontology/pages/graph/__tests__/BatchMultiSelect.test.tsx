// @vitest-environment jsdom
import "../../../../test/setup-dom.js";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import BatchMultiSelect from "../BatchMultiSelect";

function Example() {
  const [value, setValue] = useState<string[]>([]);
  return <BatchMultiSelect label="测试多选" placeholder="请选择" value={value} onChange={setValue}
    options={[{ value: "a", label: "类型 A" }, { value: "b", label: "类型 B" }]} />;
}

describe("BatchMultiSelect", () => {
  it("selects several options in one open menu and closes on Done", async () => {
    render(<Example />);
    const select = screen.getByRole("combobox", { name: "测试多选" });
    const visibleOption = (title: string) => document.querySelector(
      `.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option[title="${title}"]`,
    );
    const choose = async (title: string) => {
      const option = await waitFor(() => {
        const found = visibleOption(title);
        expect(found).not.toBeNull();
        return found as HTMLElement;
      });
      fireEvent.click(option);
    };

    fireEvent.mouseDown(select);
    await choose("类型 A");
    expect(visibleOption("类型 B")).not.toBeNull();
    await choose("类型 B");
    expect(visibleOption("类型 A")).not.toBeNull();
    expect(select.closest(".ant-select")?.textContent).toContain("类型 A");
    expect(select.closest(".ant-select")?.textContent).toContain("类型 B");

    fireEvent.click(screen.getByRole("button", { name: /完\s*成/ }));
    await waitFor(() => expect(select.getAttribute("aria-expanded")).toBe("false"));
  });
});
