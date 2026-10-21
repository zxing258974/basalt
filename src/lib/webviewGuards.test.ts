// Windows WebView2 的刷新、打印、页内查找、右键菜单要在页面里拦下;输入框的右键菜单和编辑器自己的 Ctrl+F 不受影响。
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { installWebviewGuards } from "./webviewGuards";

function press(target: EventTarget, init: KeyboardEventInit): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}

function rightClick(target: EventTarget): MouseEvent {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

describe("installWebviewGuards", () => {
  let uninstall: () => void;
  beforeEach(() => {
    uninstall = installWebviewGuards(window);
  });
  afterEach(() => {
    uninstall();
    document.body.innerHTML = "";
  });

  it("blocks reload and print even when a component stops the event", () => {
    const panel = document.createElement("div");
    panel.addEventListener("keydown", (event) => event.stopPropagation());
    document.body.append(panel);
    expect(press(panel, { key: "F5" }).defaultPrevented).toBe(true);
    expect(press(panel, { key: "r", ctrlKey: true }).defaultPrevented).toBe(true);
    expect(press(panel, { key: "R", ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(true);
    expect(press(panel, { key: "p", ctrlKey: true }).defaultPrevented).toBe(true);
    expect(press(panel, { key: "r" }).defaultPrevented).toBe(false);
    expect(press(panel, { key: "c", ctrlKey: true }).defaultPrevented).toBe(false);
  });

  it("stops the page find bar but leaves Ctrl+F to the editor that handles it", () => {
    expect(press(document.body, { key: "f", ctrlKey: true }).defaultPrevented).toBe(true);
    const editor = document.createElement("div");
    let handled = 0;
    editor.addEventListener("keydown", (event) => {
      handled += 1;
      event.preventDefault();
    });
    document.body.append(editor);
    expect(press(editor, { key: "f", ctrlKey: true }).defaultPrevented).toBe(true);
    expect(handled).toBe(1);
  });

  it("keeps the system menu in text fields only", () => {
    const input = document.createElement("input");
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    const line = document.createElement("span");
    editable.append(line);
    document.body.append(input, checkbox, editable);
    expect(rightClick(document.body).defaultPrevented).toBe(true);
    expect(rightClick(input).defaultPrevented).toBe(false);
    expect(rightClick(checkbox).defaultPrevented).toBe(true);
    expect(rightClick(line).defaultPrevented).toBe(false);
  });

  it("can be removed", () => {
    uninstall();
    expect(press(document.body, { key: "F5" }).defaultPrevented).toBe(false);
    uninstall = installWebviewGuards(window);
  });
});
