// 单选卡片:Tab 只停在选中的那张、方向键换选项、焦点从组外进来时挪到选中的那张。
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { ChoiceList, type Choice } from "./ChoiceList";

const CHOICES: Choice<string>[] = [
  { value: "a", label: "甲", code: "A", detail: "第一项" },
  { value: "b", label: "乙", detail: "第二项" },
  { value: "c", label: "丙", detail: "第三项" },
];

function Harness({ initial }: { initial: string }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <button type="button">外面</button>
      <ChoiceList label="选项" choices={CHOICES} value={value} onChange={setValue} />
    </>
  );
}

describe("ChoiceList", () => {
  it("lets Tab land on the checked card and arrows move the choice", async () => {
    const user = userEvent.setup();
    render(<Harness initial="b" />);
    const radio = (name: RegExp) => screen.getByRole("radio", { name });
    expect(radio(/^甲A/)).toHaveAttribute("tabindex", "-1");
    await user.click(screen.getByRole("button", { name: "外面" }));
    await user.tab();
    expect(radio(/^乙/)).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(radio(/^丙/)).toHaveAttribute("aria-checked", "true");
    expect(radio(/^丙/)).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    expect(radio(/^甲/)).toHaveFocus();
    await user.keyboard("{ArrowUp}{Enter}");
    expect(radio(/^丙/)).toHaveAttribute("aria-checked", "true");
  });

  it("moves focus that enters from outside to the checked card", async () => {
    render(<Harness initial="c" />);
    screen.getByRole("radio", { name: /^甲/ }).focus();
    expect(screen.getByRole("radio", { name: /^丙/ })).toHaveFocus();
    await userEvent.setup().click(screen.getByRole("radio", { name: /^乙/ }));
    expect(screen.getByRole("radio", { name: /^乙/ })).toHaveFocus();
    expect(screen.getByRole("radio", { name: /^乙/ })).toHaveAttribute("aria-checked", "true");
  });

  it("keeps focus on the card just picked when a click does not focus it (WebKit)", () => {
    render(<Harness initial="a" />);
    screen.getByRole("button", { name: "外面" }).focus();
    // WKWebView 里点按钮不会先把焦点给它:直接派发 click,焦点还在组外。
    fireEvent.click(screen.getByRole("radio", { name: /^乙/ }));
    expect(screen.getByRole("radio", { name: /^乙/ })).toHaveFocus();
    expect(screen.getByRole("radio", { name: /^乙/ })).toHaveAttribute("aria-checked", "true");
  });
});
