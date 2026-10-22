// 导入 Navicat 保存的查询:来源卡片上的条数、预览里的去向与原因、只导入新增的、导完的汇报。
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting, type Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { QueryImportCandidate } from "../../lib/queryTypes";
import { ImportDialog } from "./ImportDialog";
import {
  defaultQuerySelection,
  queryReportHeadline,
  querySelectionSummary,
  queryStatusLabel,
  sourcePath,
  targetPath,
  toggleQuery,
} from "./queryImportModel";

function setup(overrides: Partial<Backend> = {}) {
  const base = createMockBackend();
  const backend: Backend = { ...base, importNavicatQueries: vi.fn(base.importNavicatQueries), ...overrides };
  setBackendForTesting(backend);
  const onClose = vi.fn();
  render(
    <QueryClientProvider client={createQueryClient()}>
      <ImportDialog onClose={onClose} />
    </QueryClientProvider>,
  );
  return { backend, onClose, user: userEvent.setup() };
}

afterEach(() => setBackendForTesting(null));

describe("importing Navicat queries", () => {
  it("previews where each query goes and imports only the new ones", async () => {
    const { backend, onClose, user } = setup();
    const source = await screen.findByRole("radio", { name: /Navicat 保存的查询.*检测到 3 条查询/ });
    await user.click(source);

    const dialog = await screen.findByRole("dialog", { name: "导入查询" });
    const refund = within(dialog).getByRole("row", { name: /退款明细/ });
    expect(within(refund).getByText("MySQL · 本地 MySQL / shop")).toBeInTheDocument();
    expect(within(refund).getByText("本地 MySQL / shop")).toBeInTheDocument();
    expect(within(refund).getByRole("checkbox")).toBeChecked();

    const readers = within(dialog).getByRole("row", { name: /读者/ });
    expect(within(readers).getByText("按 Navicat 里同名连接的地址匹配")).toBeInTheDocument();
    expect(within(readers).getByText("FOLIO 测试库 / folio / diku_mod_users")).toBeInTheDocument();

    const core = within(dialog).getByRole("row", { name: /月度销售/ });
    expect(within(core).getByRole("checkbox")).toBeDisabled();
    expect(within(core).getByText(/没有叫「cloud-mysql」的 MySQL 连接/)).toBeInTheDocument();
    expect(within(dialog).getByText("已选 2 条，1 条不能导入")).toBeInTheDocument();

    await user.click(within(readers).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "导入 1 条查询" }));
    expect(backend.importNavicatQueries).toHaveBeenCalledWith(["0/0/MySQL/本地 MySQL/shop/退款明细.sql"]);
    expect(await within(dialog).findByText("已导入 1 条查询")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "完成" }));
    expect(onClose).toHaveBeenCalled();
    expect(await backend.listSavedQueries("mock-mysql", "shop", null)).toContainEqual(
      expect.objectContaining({ name: "退款明细" }),
    );
  });

  it("switches back to connection sources from the query preview", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("radio", { name: /Navicat 保存的查询/ }));
    await screen.findByRole("dialog", { name: "导入查询" });
    await user.click(screen.getByRole("radio", { name: /本机 Navicat 的连接/ }));
    expect(await screen.findByRole("dialog", { name: "导入连接" })).toBeInTheDocument();
  });

  it("shows a failed import and keeps the preview", async () => {
    const failure = { code: "storage", message: "写不进去", fields: [] };
    const { user } = setup({
      importNavicatQueries: vi.fn<Backend["importNavicatQueries"]>().mockRejectedValue(failure),
    });
    await user.click(await screen.findByRole("radio", { name: /Navicat 保存的查询/ }));
    const dialog = await screen.findByRole("dialog", { name: "导入查询" });
    await user.click(within(dialog).getByRole("button", { name: "导入 2 条查询" }));
    expect(await within(dialog).findByText("写不进去")).toBeInTheDocument();
    expect(within(dialog).getByRole("row", { name: /退款明细/ })).toBeInTheDocument();
  });
});

describe("query import model", () => {
  const candidate = (id: string, status: QueryImportCandidate["status"]): QueryImportCandidate => ({
    id,
    name: id,
    sourceType: "PostgreSQL",
    sourceConnection: "pg",
    sourceDatabase: "folio",
    sourceSchema: "public",
    target:
      status.kind === "unmatched"
        ? null
        : { connectionId: "p", connectionName: "PG", database: "folio", schema: "public", name: id },
    matchedBy: status.kind === "unmatched" ? null : "name",
    status,
  });
  const list = [
    candidate("a", { kind: "new" }),
    candidate("b", { kind: "exists" }),
    candidate("c", { kind: "conflict" }),
    candidate("d", { kind: "unmatched", reason: "x" }),
  ];

  it("only new queries can be picked", () => {
    expect([...defaultQuerySelection(list)]).toEqual(["a"]);
    expect([...toggleQuery(new Set(), list, "b")]).toEqual([]);
    expect([...toggleQuery(new Set(["a"]), list, "a")]).toEqual([]);
    expect(querySelectionSummary(list, new Set())).toBe("未选择查询，3 条不能导入");
  });

  it("words statuses, places and the report", () => {
    expect(list.map((c) => queryStatusLabel(c.status))).toEqual(["新增", "已导入", "同名冲突", "无法导入"]);
    expect(sourcePath(list[0]!)).toBe("PostgreSQL · pg / folio / public");
    expect(targetPath(list[0]!)).toBe("PG / folio / public");
    expect(targetPath(list[3]!)).toBeNull();
    expect(queryReportHeadline({ imported: [], skipped: [{ name: "a", reason: "r" }] })).toBe(
      "没有导入任何查询，跳过 1 条",
    );
  });
});
