// 导入向导的主流程(mock 后端):选文件、改编码分隔符、调映射、清空表的二次确认、进度与取消、失败明细另存。
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
    const mapping = view.getByRole("table", { name: "列映射" });
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
    await user.click(view.getByRole("checkbox", { name: /导入前清空/ }));
    await user.click(view.getByRole("button", { name: "开始导入" }));
    const confirm = within(await screen.findByRole("dialog", { name: "清空表再导入？" }));
    expect(confirm.getByText(/每批单独提交/)).toBeInTheDocument();
    await user.click(confirm.getByRole("button", { name: "清空并导入" }));

    expect(await view.findByText("导入完成，有些行失败了")).toBeInTheDocument();
    expect(importData.mock.calls[0]![1]).toMatchObject({
      connectionId: "mock-mysql",
      table: { database: "shop", table: "customers" },
      source: { path: "/Users/me/Downloads/customers.csv", hasHeader: true },
      options: { onError: "skip", batchSize: 200, clearTable: true, binary: "hex" },
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
    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("checkbox", { name: /导入前清空/ }));
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
          inserted: 0,
          failed: 0,
          failures: [],
          summary: "已取消。整个导入已经回滚，表没有变化。",
        });
        return Promise.resolve(true);
      }),
    };
    const { user, view } = await importCustomers(controlled);
    await pickFile(user);
    await user.click(view.getByRole("button", { name: "下一步" }));
    await user.click(view.getByRole("button", { name: "下一步" }));
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

  it("switches sheets of an Excel file", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "pickImportFile").mockResolvedValue("/Users/me/Downloads/book.xlsx");
    const preview = vi.spyOn(backend, "previewImport");
    const { user, view } = await importCustomers(backend);
    await pickFile(user);
    expect(view.getByText("Excel · 工作表「Sheet1」")).toBeInTheDocument();
    await user.selectOptions(view.getByRole("combobox", { name: "工作表" }), "说明");
    await waitFor(() => expect(preview.mock.calls[1]![2].format).toEqual({ kind: "excel", sheet: "说明" }));
  });
});
