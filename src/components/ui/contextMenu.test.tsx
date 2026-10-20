import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ContextMenu, type MenuEntry } from "./ContextMenu";

function renderMenu() {
  const onClose = vi.fn();
  const onCopy = vi.fn();
  const items: MenuEntry[] = [
    { label: "打开", onSelect: vi.fn() },
    { label: "刷新", disabled: true, onSelect: vi.fn() },
    "separator",
    { label: "复制", shortcut: "⌘D", onSelect: onCopy },
    { label: "删除", danger: true, onSelect: vi.fn() },
  ];
  render(
    <>
      <button>外面</button>
      <ContextMenu x={10} y={20} items={items} onClose={onClose} />
    </>,
  );
  return { onClose, onCopy, user: userEvent.setup() };
}

describe("ContextMenu", () => {
  it("focuses the first enabled item and skips disabled ones with the arrow keys", async () => {
    const { user } = renderMenu();
    expect(screen.getByRole("menuitem", { name: "打开" })).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("menuitem", { name: /^复制/ })).toHaveFocus();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(screen.getByRole("menuitem", { name: "打开" })).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(screen.getByRole("menuitem", { name: "删除" })).toHaveFocus();
  });

  it("closes before running the chosen item", async () => {
    const { user, onClose, onCopy } = renderMenu();
    await user.click(screen.getByRole("menuitem", { name: /^复制/ }));
    expect(onClose).toHaveBeenCalledOnce();
    expect(onCopy).toHaveBeenCalledOnce();
    expect(onClose.mock.invocationCallOrder[0]).toBeLessThan(onCopy.mock.invocationCallOrder[0]!);
  });

  it("closes on Escape, a click outside, or the window losing focus", async () => {
    const { user, onClose } = renderMenu();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("menuitem", { name: "删除" }).closest("[role=menu]")!);
    expect(onClose).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "外面" }));
    act(() => void window.dispatchEvent(new Event("blur")));
    expect(onClose).toHaveBeenCalledTimes(3);
  });

  it("marks separators and dangerous items", () => {
    renderMenu();
    expect(screen.getByRole("separator")).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "删除" })).toHaveClass("is-danger");
    expect(screen.getByRole("menuitem", { name: "刷新" })).toBeDisabled();
  });
});
