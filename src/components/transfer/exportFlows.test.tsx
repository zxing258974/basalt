// 导出的主流程(mock 后端):从对象树、表数据页、查询结果打开,选格式、看进度、取消、完成后在访达中显示。
import { screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { ExportRequest, TransferProgress } from "../../lib/transferTypes";
import type { CommandError } from "../../lib/types";
import { useTransfer } from "../../stores/transfer";
import { findEditor, installLayoutStubs, setSql } from "../../test/editor";
import { renderApp } from "../../test/renderApp";

installLayoutStubs();

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const findRow = async (name: string) =>
  (await tree().findByRole("treeitem", { name })).firstElementChild as HTMLElement;
const dialog = (name: string) => screen.findByRole("dialog", { name });

async function openShop(backend?: Backend) {
  const app = renderApp(backend);
  const { user } = app;
  await user.dblClick(await findRow("开发"));
  await user.dblClick(await findRow("本地 MySQL"));
  await user.click(await findRow("shop"));
  await user.keyboard("{ArrowRight}");
  await user.dblClick(await findRow("表"));
  await findRow("customers");
  useTransfer.setState({ dialog: null });
  return app;
}

async function exportCustomers(backend?: Backend) {
  const app = await openShop(backend);
  await app.user.pointer({ keys: "[MouseRight]", target: await findRow("customers") });
  await app.user.click(screen.getByRole("menuitem", { name: "导出数据…" }));
  return { ...app, view: within(await dialog("导出数据")) };
}

/** 进度、结果都由测试控制的后端:导出停在半路,等测试放行或取消。 */
function controlledBackend() {
  const backend = createMockBackend();
  const listeners = new Set<(p: TransferProgress) => void>();
  let release: (() => void) | null = null;
  let reject: ((error: CommandError) => void) | null = null;
  const controlled: Backend = {
    ...backend,
    onTransferProgress: (handler) => {
      listeners.add(handler);
      return Promise.resolve(() => void listeners.delete(handler));
    },
    exportData: (jobId, request) =>
      new Promise((resolve, fail) => {
        for (const listener of listeners)
          listener({ jobId, phase: "exporting", rows: 12_345, failed: 0, fraction: null });
        // 别的任务的进度不该被认领。
        for (const listener of listeners)
          listener({ jobId: "other", phase: "exporting", rows: 9, failed: 0, fraction: null });
        release = () => resolve({ rows: 20_000, bytes: 2_048_000, path: request.path, notice: "重新连上过" });
        reject = fail;
      }),
    cancelTransfer: vi.fn(() => {
      reject?.({ code: "cancelled", message: "已取消", fields: [] });
      return Promise.resolve(true);
    }),
  };
  return { backend: controlled, release: () => release?.() };
}

describe("exporting", { timeout: 15_000 }, () => {
  it("exports a table as JSON Lines and reveals the file", async () => {
    const backend = createMockBackend();
    const exportData = vi.spyOn(backend, "exportData");
    const reveal = vi.spyOn(backend, "revealInFinder");
    const { user, view } = await exportCustomers(backend);
    expect(view.getByRole("radio", { name: "整张表" })).toHaveAttribute("aria-checked", "true");
    expect(view.getByRole("radio", { name: /当前筛选条件下的结果/ })).toBeDisabled();

    await user.click(view.getByRole("radio", { name: /JSON/ }));
    await user.selectOptions(view.getByRole("combobox", { name: "写法" }), "lines");
    await user.selectOptions(view.getByRole("combobox", { name: "二进制写成" }), "base64");
    await user.click(view.getByRole("button", { name: "导出…" }));

    const done = within(await dialog("导出完成"));
    expect(done.getByText(/已导出/)).toHaveTextContent("已导出 48,000 行，文件 4.4 MB");
    expect(done.getByText("customers.jsonl")).toBeInTheDocument();
    const request = exportData.mock.calls[0]![1];
    expect(request).toEqual<ExportRequest>({
      connectionId: "mock-mysql",
      source: {
        kind: "table",
        table: { database: "shop", schema: null, table: "customers" },
        filter: null,
        sort: null,
      },
      options: { format: { kind: "json", lines: true }, binary: "base64" },
      path: "/Users/me/Downloads/customers.jsonl",
    });
    await user.click(done.getByRole("button", { name: "在访达中显示" }));
    expect(reveal).toHaveBeenCalledWith("/Users/me/Downloads/customers.jsonl");
    await user.click(done.getByRole("button", { name: "完成" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("table data exports the current filter by default", async () => {
    const backend = createMockBackend();
    const exportData = vi.spyOn(backend, "exportData");
    const { user } = await openShop(backend);
    await user.dblClick(await findRow("customers"));
    const filter = await screen.findByRole("textbox", { name: "筛选条件（WHERE 子句）" });
    await user.type(filter, "city = '上海'{Enter}");
    await user.click(screen.getByRole("button", { name: "导出" }));
    const view = within(await dialog("导出数据"));
    expect(view.getByRole("radio", { name: /当前筛选条件下的结果/ })).toHaveAttribute("aria-checked", "true");
    expect(view.getByText("city = '上海'")).toBeInTheDocument();
    await user.click(view.getByRole("radio", { name: /SQL/ }));
    expect(view.getByRole("textbox", { name: "表名" })).toHaveValue("customers");
    await user.click(view.getByRole("button", { name: "导出…" }));
    await dialog("导出完成");
    const request = exportData.mock.calls[0]![1];
    expect(request.source).toMatchObject({ kind: "table", filter: "city = '上海'" });
    expect(request.options.format).toEqual({ kind: "sql", table: "customers", rowsPerStatement: 100 });
  });

  it("checks the INSERT table name and does nothing when the save dialog is cancelled", async () => {
    const backend = createMockBackend();
    const exportData = vi.spyOn(backend, "exportData");
    vi.spyOn(backend, "pickSavePath").mockResolvedValueOnce(null);
    const { user, view } = await exportCustomers(backend);
    await user.click(view.getByRole("button", { name: "导出…" }));
    expect(exportData).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "导出数据" })).toBeInTheDocument();

    await user.click(view.getByRole("radio", { name: /SQL/ }));
    await user.clear(view.getByRole("textbox", { name: "表名" }));
    await user.click(view.getByRole("button", { name: "导出…" }));
    expect(view.getByRole("alert")).toHaveTextContent("请填写 INSERT 语句里的表名。");
  });

  it("shows progress for its own job, cancels, and reports failures", async () => {
    const { backend } = controlledBackend();
    const { user, view } = await exportCustomers(backend);
    await user.click(view.getByRole("button", { name: "导出…" }));
    const status = await view.findByRole("status");
    expect(within(status).getByText("12,345")).toBeInTheDocument();
    expect(within(status).getByText("正在导出")).toBeInTheDocument();
    // 进行中按 Esc 不关。
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog", { name: "导出数据" })).toBeInTheDocument();

    await user.click(within(status).getByRole("button", { name: "取消" }));
    expect(backend.cancelTransfer).toHaveBeenCalledTimes(1);
    expect(await view.findByText("已取消，没有生成文件。")).toBeInTheDocument();
    expect(view.queryByRole("alert")).not.toBeInTheDocument();

    vi.spyOn(backend, "exportData").mockRejectedValueOnce({
      code: "invalid_data",
      message: "第 2 行「name」里的「😀」GBK 表示不了",
      fields: [],
    });
    await user.click(view.getByRole("button", { name: "导出…" }));
    expect(await view.findByRole("alert")).toHaveTextContent("GBK 表示不了");
  });

  it("shows the notice from the backend when it finishes", async () => {
    const { backend, release } = controlledBackend();
    const { user, view } = await exportCustomers(backend);
    await user.click(view.getByRole("button", { name: "导出…" }));
    await view.findByRole("status");
    release();
    const done = within(await dialog("导出完成"));
    expect(done.getByText("重新连上过")).toBeInTheDocument();
    expect(done.getByText(/已导出/)).toHaveTextContent("20,000 行，文件 2.0 MB");
  });

  it("exports a shown query result without running it again", async () => {
    const backend = createMockBackend();
    const exportData = vi.spyOn(backend, "exportData");
    const { user } = renderApp(backend);
    await user.dblClick(await findRow("开发"));
    await user.dblClick(await findRow("本地 MySQL"));
    await user.pointer({ keys: "[MouseRight]", target: await findRow("shop") });
    await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
    setSql(await findEditor("查询 1"), "SELECT * FROM customers LIMIT 5");
    await user.click(screen.getByRole("button", { name: /^执行/ }));
    await screen.findByRole("grid", { name: "查询 1 的结果 1" });
    await user.click(screen.getByRole("button", { name: "导出" }));
    const view = within(await dialog("导出数据"));
    expect(view.getByText("导出执行时读到的全部 5 行。")).toBeInTheDocument();
    await user.click(view.getByRole("button", { name: "导出…" }));
    await dialog("导出完成");
    const request = exportData.mock.calls[0]![1];
    expect(request.source.kind).toBe("result");
    expect(request.path).toBe("/Users/me/Downloads/customers.csv");
  });
});
