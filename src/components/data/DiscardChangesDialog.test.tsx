import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { resetStores } from "../../test/renderApp";
import { useUi } from "../../stores/ui";
import { DiscardChangesDialog } from "./DiscardChangesDialog";

beforeEach(resetStores);

describe("DiscardChangesDialog", () => {
  it("offers to close when tabs or connections are closing", () => {
    useUi.getState().askDiscard({ count: 1, queries: 0, designs: 0, onConfirm: vi.fn() });
    render(<DiscardChangesDialog />);
    expect(screen.getByRole("button", { name: "放弃修改并关闭" })).toBeInTheDocument();
    expect(screen.getByText("有 1 行修改还没有提交，关闭后就没有了。")).toBeInTheDocument();
  });

  it("offers to quit when the whole app is quitting", async () => {
    const onConfirm = vi.fn();
    useUi.getState().askDiscard({ count: 0, queries: 2, designs: 1, quit: true, onConfirm });
    render(<DiscardChangesDialog />);
    expect(screen.getByText("有 2 个查询还没有保存、1 张表的结构修改还没有保存，退出后就没有了。")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "放弃修改并退出" }));
    expect(onConfirm).toHaveBeenCalledOnce();
    expect(useUi.getState().discardPrompt).toBeNull();
  });

  it("stays put when cancelled", async () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    useUi.getState().askDiscard({ count: 3, queries: 0, designs: 0, quit: true, onConfirm, onCancel });
    render(<DiscardChangesDialog />);

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();
    expect(useUi.getState().discardPrompt).toBeNull();
  });
});
