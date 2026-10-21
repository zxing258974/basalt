// 面板分隔条:拖动方向跟着分隔条在哪一边走,上下限、给旁边留宽、方向键、双击恢复、记在本机。
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { REST_MIN, usePanelWidth, type PanelWidthOptions } from "./usePanelWidth";

const RIGHT_PANEL: PanelWidthOptions = {
  storageKey: "test.panelWidth",
  defaultWidth: 300,
  min: 240,
  max: 600,
  edge: "left",
  label: "调整面板宽度",
};

/** 和真实布局一样:能伸缩的一块、分隔条、面板。 */
function Harness({ options }: { options: PanelWidthOptions }) {
  const { width, separator } = usePanelWidth(options);
  const parts = [
    <div key="rest" data-testid="rest" />,
    <div key="separator" {...separator} />,
    <div key="panel" data-testid="panel" style={{ width }} />,
  ];
  return <div>{options.edge === "left" ? parts : parts.reverse()}</div>;
}

const separator = () => screen.getByRole("separator", { name: /调整/ });
const value = () => Number(separator().getAttribute("aria-valuenow"));

function dragBy(dx: number) {
  fireEvent.pointerDown(separator(), { clientX: 500, pointerId: 1 });
  fireEvent.pointerMove(separator(), { clientX: 500 + dx });
  fireEvent.pointerUp(separator());
}

function layOut(rest: number, panel: number) {
  screen.getByTestId("rest").getBoundingClientRect = () => ({ width: rest }) as DOMRect;
  screen.getByTestId("panel").getBoundingClientRect = () => ({ width: panel }) as DOMRect;
}

describe("usePanelWidth", () => {
  beforeEach(() => {
    localStorage.clear();
    HTMLElement.prototype.setPointerCapture = vi.fn();
  });
  afterEach(() => vi.restoreAllMocks());

  it("widens a right-hand panel when dragged left and remembers it", () => {
    render(<Harness options={RIGHT_PANEL} />);
    expect(value()).toBe(300);
    expect(separator()).toHaveAttribute("tabindex", "0");
    dragBy(-80);
    expect(value()).toBe(380);
    expect(localStorage.getItem("test.panelWidth")).toBe("380");
    dragBy(1000);
    expect(value()).toBe(240);
    dragBy(-1000);
    expect(value()).toBe(600);
  });

  it("widens a left-hand panel when dragged right", () => {
    render(<Harness options={{ ...RIGHT_PANEL, edge: "right" }} />);
    dragBy(40);
    expect(value()).toBe(340);
  });

  it("leaves room for the area next to it", () => {
    render(<Harness options={RIGHT_PANEL} />);
    layOut(REST_MIN + 50, 300);
    dragBy(-200);
    expect(value()).toBe(350);
  });

  it("keeps that room when sized with the keyboard too", () => {
    render(<Harness options={RIGHT_PANEL} />);
    layOut(REST_MIN + 100, 300);
    fireEvent.keyDown(separator(), { key: "End" });
    expect(value()).toBe(400);
    fireEvent.keyDown(separator(), { key: "ArrowLeft", shiftKey: true });
    expect(value()).toBe(364);
  });

  it("starts from the width actually shown when the window squeezed the panel", () => {
    localStorage.setItem("test.panelWidth", "560");
    render(<Harness options={RIGHT_PANEL} />);
    layOut(REST_MIN, 400);
    dragBy(20);
    expect(value()).toBe(380);
  });

  it("moves with the arrow keys and resets on double click", () => {
    render(<Harness options={RIGHT_PANEL} />);
    fireEvent.keyDown(separator(), { key: "ArrowLeft" });
    expect(value()).toBe(316);
    fireEvent.keyDown(separator(), { key: "ArrowRight", shiftKey: true });
    expect(value()).toBe(252);
    fireEvent.keyDown(separator(), { key: "End" });
    expect(value()).toBe(600);
    fireEvent.keyDown(separator(), { key: "Home" });
    expect(value()).toBe(240);
    expect(localStorage.getItem("test.panelWidth")).toBe("240");
    fireEvent.keyDown(separator(), { key: "a" });
    expect(value()).toBe(240);
    fireEvent.doubleClick(separator());
    expect(value()).toBe(300);
    expect(localStorage.getItem("test.panelWidth")).toBe("300");
  });

  it.each([
    ["420", 420],
    ["9999", 600],
    ["10", 240],
    ["abc", 300],
  ])("starts from the saved width %s", (saved, width) => {
    localStorage.setItem("test.panelWidth", saved);
    render(<Harness options={RIGHT_PANEL} />);
    expect(value()).toBe(width);
  });

  it("still works when the width cannot be stored", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    render(<Harness options={RIGHT_PANEL} />);
    expect(value()).toBe(300);
    dragBy(-20);
    expect(value()).toBe(320);
    expect(warn).toHaveBeenCalled();
  });
});
