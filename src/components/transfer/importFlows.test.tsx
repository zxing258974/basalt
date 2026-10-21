// 导入向导的主流程(mock 后端):选文件、选工作表、导入模式与匹配键、调映射、清空表的二次确认、进度与取消、失败明细另存。
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { ImportReport, TransferProgress } from "../../lib/transferTypes";
import { useTransfer } from "../../stores/transfer";
import { renderApp } from "../../test/renderApp";

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const findRow = async (name: string) =>
  (await tree().findByRole("treeitem", { name })).firstElementChild as HTMLElement;

async function importCustomers(backend: Backend = createMockBackend()) {
  const app = renderApp(backend);
  const { user } = app;
  useTransfer.setState({ dialog: null });
  await user.dblClick(await findRow("开发"));
  await user.dblClick(await findRow("本地 MySQL"));
  await user.click(await findRow("shop"));
  await user.keyboard("{ArrowRight}");
  await user.dblClick(await findRow("表"));
  await user.pointer({ keys: "[MouseRight]", target: await findRow("customers") });
  await user.click(screen.getByRole("menuitem", { name: "导入数据…" }));
  const view = within(await screen.findByRole("dialog", { name: "导入数据" }));
  return { ...app, view };
}

async function pickFile(user: ReturnType<typeof renderApp>["user"]) {
  const view = within(screen.getByRole("dialog", { name: "导入数据" }));
  await user.click(view.getByRole("button", { name: "选择文件…" }));
  await view.findByRole("table", { name: "文件预览" });
}

describe("importing", { timeout: 15_000 }, () => {
  it("walks through file, mapping and options, then saves the failure report", async () => {
    const backend = createMockBackend();
    const preview = vi.spyOn(backend, "previewImport");
    const importData = vi.spyOn(backend, "importData");
    const save = vi.spyOn(backend, "saveImportFailures");
    const { user, view } = await importCustomers(backend);
    expect(view.getByRole("button", { name: "下一步" })).toBeDisabled();

    await pickFile(user);
    expect(view.getByText("CSV · UTF-8 · 逗号分隔")).toBeInTheDocument();
    await user.selectOptions(view.getByRole("combobox", { name: "分隔符" }), ";");
    await waitFor(() => expect(preview).toHaveBeenCalledTimes(2));
    expect(preview.mock.calls[1]![2].format).toMatchObject({ kind: "csv", delimiter: ";" });
    await user.click(view.getByRole("checkbox", { name: "第一行是列名" }));
    await waitFor(() => expect(preview.mock.calls[2]![2].hasHeader).toBe(false));
    await user.click(view.getByRole("checkbox", { name: "第一行是列名" }));
    await waitFor(() => expect(preview).toHaveBeenCalledTimes(4));

    await user.click(view.getByRole("button", { name: "下一步" }));
    expect(view.getByRole("radio", { name: /^追加把/ })).toHaveAttribute("aria-checked", "true");
    await user.click(view.getByRole("radio", { name: /^复制/ }));
    await user.click(view.getByRole("button", { name: "下一步" }));
    const mapping = view.getByRole("table", { name: "列映射" });
    expect(within(mapping).queryByRole("columnheader", { name: "匹配键" })).not.toBeInTheDocument();
    expect(within(mapping).getByRole("combobox", { name: "name 的来源" })).toHaveDisplayValue("NAME");
    expect(within(mapping).getByRole("combobox", { name: "note 的来源" })).toHaveDisplayValue("不导入");
    await user.selectOptions(within(mapping).getByRole("combobox", { name: "name 的来源" }), "skip");
    expect(view.getByRole("note")).toHaveTextContent("「name」不能为空");
    expect(within(view.getByRole("table", { name: "按映射预览" })).queryByText("name")).not.toBeInTheDocument();
    await user.click(view.getByRole("button", { name: "恢复自动匹配" }));
    expect(view.queryByRole("note")).not.toBeInTheDocument();

    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("radio", { name: /跳过并记录/ }));
    const batch = view.getByRole("spinbutton", { name: "每批的行数" });
    await user.clear(batch);
    await user.type(batch, "200");
    await user.click(view.getByRole("button", { name: "开始导入" }));
    const confirm = within(await screen.findByRole("dialog", { name: "清空表再导入？" }));
    expect(confirm.getByText(/清空和导入在同一个事务里/)).toBeInTheDocument();
    await user.click(confirm.getByRole("button", { name: "清空并导入" }));

    expect(await view.findByText("导入完成，有些行失败了")).toBeInTheDocument();
    expect(importData.mock.calls[0]![1]).toMatchObject({
      connectionId: "mock-mysql",
      table: { database: "shop", table: "customers" },
      source: { path: "/Users/me/Downloads/customers.csv", hasHeader: true },
      options: { onError: "skip", batchSize: 200, mode: "replace", keyColumns: [], binary: "hex" },
    });
    const failures = view.getByRole("table", { name: "失败的行" });
    expect(within(failures).getByText("不是有效的整数")).toBeInTheDocument();
    expect(within(failures).getByText("（整行）")).toBeInTheDocument();
    await user.click(view.getByRole("button", { name: "导出失败明细…" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith("/Users/me/Downloads/customers-导入失败.csv", expect.any(Array)),
    );
    expect(view.getByText(/已保存到/)).toBeInTheDocument();
    await user.click(view.getByRole("button", { name: "完成" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("can go back, and declining the clear confirmation runs nothing", async () => {
    const backend = createMockBackend();
    const importData = vi.spyOn(backend, "importData");
    const { user, view } = await importCustomers(backend);
    await pickFile(user);
    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("button", { name: "上一步" }));
    expect(view.getByRole("table", { name: "文件预览" })).toBeInTheDocument();
    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("radio", { name: /^复制/ }));
    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("button", { name: "开始导入" }));
    const confirm = within(await screen.findByRole("dialog", { name: "清空表再导入？" }));
    await user.click(confirm.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog", { name: "清空表再导入？" })).not.toBeInTheDocument();
    expect(importData).not.toHaveBeenCalled();
    await user.click(view.getByRole("button", { name: "上一步" }));
    expect(view.getByRole("table", { name: "列映射" })).toBeInTheDocument();
  });

  it("shows progress and cancels a running import", async () => {
    const backend = createMockBackend();
    const listeners = new Set<(p: TransferProgress) => void>();
    let finish: ((report: ImportReport) => void) | null = null;
    const controlled: Backend = {
      ...backend,
      onTransferProgress: (handler) => {
        listeners.add(handler);
        return Promise.resolve(() => void listeners.delete(handler));
      },
      importData: (jobId) =>
        new Promise((resolve) => {
          for (const l of listeners) l({ jobId, phase: "importing", rows: 600, failed: 3, fraction: 0.5 });
          finish = resolve;
        }),
      cancelTransfer: vi.fn(() => {
        finish?.({
          status: "cancelled",
          affected: 0,
          unmatched: 0,
          failed: 0,
          failures: [],
          summary: "已取消。整个导入已经回滚，表没有变化。",
        });
        return Promise.resolve(true);
      }),
    };
    const { user, view } = await importCustomers(controlled);
    await pickFile(user);
    for (let step = 0; step < 3; step += 1) await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("button", { name: "开始导入" }));
    const status = await view.findByRole("status");
    expect(within(status).getByText("600")).toBeInTheDocument();
    expect(within(status).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "50");
    expect(within(status).getByText(/其中 3 行失败/)).toBeInTheDocument();
    await user.click(within(status).getByRole("button", { name: "取消" }));
    expect(await view.findByText("已取消")).toBeInTheDocument();
    expect(view.getByText(/整个导入已经回滚/)).toBeInTheDocument();
    expect(view.queryByRole("button", { name: "导出失败明细…" })).not.toBeInTheDocument();
  });

  it("explains files it cannot read", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "previewImport").mockRejectedValueOnce({
      code: "invalid_data",
      message: "文件第 12 个字节附近有不是 UTF-8 的内容，请换一种编码再试。",
      fields: [],
    });
    const { user, view } = await importCustomers(backend);
    await user.click(view.getByRole("button", { name: "选择文件…" }));
    expect(await view.findByRole("alert")).toHaveTextContent("换一种编码");
    expect(view.getByRole("button", { name: "下一步" })).toBeDisabled();
    await user.click(view.getByRole("button", { name: "换一个文件…" }));
    await view.findByRole("table", { name: "文件预览" });
    expect(view.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("makes the user pick one sheet of a workbook that has several", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "pickImportFile").mockResolvedValue("/Users/me/Downloads/book.xlsx");
    const preview = vi.spyOn(backend, "previewImport");
    const { user, view } = await importCustomers(backend);
    await user.click(view.getByRole("button", { name: "选择文件…" }));
    const sheets = within(await view.findByRole("radiogroup", { name: "工作表" }));
    expect(view.getByText(/有 2 个工作表，先选要导入的那一个/)).toBeInTheDocument();
    expect(view.queryByRole("table", { name: "文件预览" })).not.toBeInTheDocument();
    expect(view.getByRole("button", { name: "下一步" })).toBeDisabled();

    await user.click(sheets.getByRole("radio", { name: "说明" }));
    await waitFor(() => expect(preview.mock.calls[1]![2].format).toEqual({ kind: "excel", sheet: "说明" }));
    expect(await view.findByRole("table", { name: "文件预览" })).toBeInTheDocument();
    expect(sheets.getByRole("radio", { name: "说明" })).toHaveAttribute("aria-checked", "true");
    expect(view.getByRole("button", { name: "下一步" })).toBeEnabled();

    // 选回第一个:已经读过的就不必再读,但仍然算选过了。
    await user.click(sheets.getByRole("radio", { name: "Sheet1" }));
    await waitFor(() => expect(preview).toHaveBeenCalledTimes(3));
    expect(view.getByRole("button", { name: "下一步" })).toBeEnabled();
  });

  it("loads the sheet clicked last even if an earlier preview is still loading", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "pickImportFile").mockResolvedValue("/Users/me/Downloads/book.xlsx");
    const real = backend.previewImport.bind(backend);
    const pending: (() => void)[] = [];
    const preview = vi.spyOn(backend, "previewImport").mockImplementation(async (...args) => {
      const result = await real(...args);
      // 第一次(选文件)立刻返回;之后的每次都等测试放行。
      if (preview.mock.calls.length > 1) await new Promise<void>((resolve) => pending.push(resolve));
      return result;
    });
    const { user, view } = await importCustomers(backend);
    await user.click(view.getByRole("button", { name: "选择文件…" }));
    const sheets = within(await view.findByRole("radiogroup", { name: "工作表" }));
    await user.click(sheets.getByRole("radio", { name: "说明" }));
    await user.click(sheets.getByRole("radio", { name: "Sheet1" }));
    await waitFor(() => expect(preview).toHaveBeenCalledTimes(3));
    expect(preview.mock.calls[2]![2].format).toEqual({ kind: "excel", sheet: "Sheet1" });
    for (const resolve of pending) resolve();
    expect(await view.findByText("Excel · 工作表「Sheet1」")).toBeInTheDocument();
  });

  it("matches rows by the primary key when updating", async () => {
    const backend = createMockBackend();
    const importData = vi.spyOn(backend, "importData");
    const { user, view } = await importCustomers(backend);
    await pickFile(user);
    await user.click(view.getByRole("button", { name: "下一步" }));
    expect(view.getByText(/能用来匹配的键：主键（id）/)).toBeInTheDocument();
    await user.click(view.getByRole("radio", { name: /^更新/ }));
    await user.click(view.getByRole("button", { name: "下一步" }));

    const mapping = within(view.getByRole("table", { name: "列映射" }));
    expect(mapping.getByRole("checkbox", { name: "按 id 匹配" })).toBeChecked();
    await user.click(mapping.getByRole("checkbox", { name: "按 id 匹配" }));
    expect(view.getByRole("note")).toHaveTextContent("要勾选匹配键");
    expect(view.getByRole("button", { name: "下一步" })).toBeDisabled();
    await user.click(mapping.getByRole("checkbox", { name: "按 email 匹配" }));
    expect(view.queryByRole("note")).not.toBeInTheDocument();

    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("button", { name: "开始导入" }));
    expect(await view.findByText(/更新完成/)).toBeInTheDocument();
    expect(view.getByText(/未匹配/)).toHaveTextContent("未匹配 12 行");
    expect(importData.mock.calls[0]![1].options).toMatchObject({ mode: "update", keyColumns: ["email"] });
  });

  it("only offers upsert on unique keys and confirms deleting", async () => {
    const backend = createMockBackend();
    const importData = vi.spyOn(backend, "importData");
    const { user, view } = await importCustomers(backend);
    await pickFile(user);
    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("radio", { name: /^追加或更新/ }));
    await user.click(view.getByRole("button", { name: "下一步" }));
    const mapping = within(view.getByRole("table", { name: "列映射" }));
    expect(mapping.getByRole("checkbox", { name: "按 id 匹配" })).toBeChecked();
    await user.click(mapping.getByRole("checkbox", { name: "按 name 匹配" }));
    expect(view.getByRole("note")).toHaveTextContent("只能按主键或一个唯一索引");
    await user.click(within(view.getByRole("group", { name: "可用的键" })).getByRole("button", { name: /主键/ }));
    expect(mapping.getByRole("checkbox", { name: "按 name 匹配" })).not.toBeChecked();

    await user.click(view.getByRole("button", { name: "上一步" }));
    await user.click(view.getByRole("radio", { name: /^删除/ }));
    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("button", { name: "开始导入" }));
    const confirm = within(await screen.findByRole("dialog", { name: "按文件删除表里的行？" }));
    await user.click(confirm.getByRole("button", { name: "删除" }));
    expect(await view.findByText(/删除完成/)).toBeInTheDocument();
    expect(importData.mock.calls[0]![1].options).toMatchObject({ mode: "delete", keyColumns: ["id"] });
  });

  it("cannot upsert into a table without a primary key or unique index", async () => {
    const backend = createMockBackend();
    const real = backend.previewImport.bind(backend);
    vi.spyOn(backend, "previewImport").mockImplementation(async (...args) => ({
      ...(await real(...args)),
      uniqueKeys: [],
    }));
    const { user, view } = await importCustomers(backend);
    await pickFile(user);
    await user.click(view.getByRole("button", { name: "下一步" }));
    expect(view.getByRole("radio", { name: /^追加或更新/ })).toBeDisabled();
    expect(view.getByText(/没有主键，也没有唯一索引/)).toBeInTheDocument();
  });
});
