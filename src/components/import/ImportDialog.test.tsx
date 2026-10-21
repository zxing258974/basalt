// 导入对话框:来源检测与选择、默认勾选、导入后的汇报与 .ncx 删除提醒。
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting, type Backend } from "../../lib/api";
import { setPlatformForTesting } from "../../lib/platform";
import { createMockBackend } from "../../lib/mockBackend";
import type { ImportReport } from "../../lib/importTypes";
import type { CommandError, ConnectionConfig } from "../../lib/types";
import { ImportDialog } from "./ImportDialog";

const NCX_PATH = "/Users/me/Downloads/connections.ncx";

function setup(overrides: Partial<Backend> = {}, seed?: ConnectionConfig[]) {
  const base = createMockBackend(seed);
  const backend: Backend = {
    ...base,
    scanImportSource: vi.fn(base.scanImportSource),
    importConnections: vi.fn(base.importConnections),
    pickNcxFile: vi.fn<Backend["pickNcxFile"]>(() => Promise.resolve(NCX_PATH)),
    ...overrides,
  };
  setBackendForTesting(backend);
  const onClose = vi.fn();
  const client = createQueryClient();
  const user = userEvent.setup();
  render(
    <QueryClientProvider client={client}>
      <ImportDialog onClose={onClose} />
    </QueryClientProvider>,
  );
  return { backend, onClose, user };
}

const row = (name: string) => screen.getByRole("row", { name: new RegExp(name) });

afterEach(() => setBackendForTesting(null));

describe("import dialog", () => {
  it("detects local Navicat and previews it with only new connections checked", async () => {
    setup();

    expect(await screen.findByRole("radio", { name: /本机 Navicat.*检测到 5 个连接/ })).toBeChecked();
    expect(await screen.findByRole("checkbox", { name: "选择「订单库」" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "选择「本地 MySQL」" })).not.toBeChecked();
    expect(within(row("本地 MySQL")).getByText("已存在")).toBeInTheDocument();
    expect(within(row("本地 MySQL")).getByText("与「本地 MySQL」重复")).toBeInTheDocument();

    const redis = screen.getByRole("checkbox", { name: "选择「会话缓存」" });
    expect(redis).toBeDisabled();
    expect(within(row("会话缓存")).getByText(/暂不支持 Redis/)).toBeInTheDocument();
    expect(within(row("测试库")).getByText("隧道 / SSL 设置未迁移（M4 支持）")).toBeInTheDocument();
    expect(screen.getByText(/密码存在它自己的钥匙串里/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "导入 3 个连接" })).toBeEnabled();
  });

  it("moves focus to the import button once the preview is ready, not to a source card", async () => {
    setup();
    const importButton = await screen.findByRole("button", { name: "导入 3 个连接" });
    await waitFor(() => expect(importButton).toHaveFocus());
  });

  it("imports the checked connections and reports the result", async () => {
    const { backend, user } = setup();
    await user.click(await screen.findByRole("checkbox", { name: "选择「测试库」" }));
    await user.click(screen.getByRole("button", { name: "导入 2 个连接" }));

    expect(await screen.findByText("已导入 2 个连接")).toBeInTheDocument();
    expect(backend.importConnections).toHaveBeenCalledWith({ kind: "localNavicat" }, [
      "MySQL:订单库",
      "PostgreSQL:分析库",
    ]);
    expect(screen.getAllByText("首次打开时输入密码")).toHaveLength(2);
    expect(screen.queryByText(/请删除/)).not.toBeInTheDocument();
    const names = (await backend.listConnections()).map((c) => c.name);
    expect(names).toEqual(expect.arrayContaining(["订单库", "分析库"]));
  });

  it("previews a picked .ncx file and reminds to delete it after importing", async () => {
    const { backend, user } = setup();
    await screen.findByRole("checkbox", { name: "选择「订单库」" });

    await user.click(screen.getByRole("radio", { name: /Navicat 导出文件/ }));
    expect(await screen.findByRole("radio", { name: /connections\.ncx/ })).toBeChecked();
    expect(backend.scanImportSource).toHaveBeenLastCalledWith({ kind: "ncxFile", path: NCX_PATH });
    expect(await screen.findByRole("checkbox", { name: "选择「报表 PG」" })).toBeChecked();
    expect(within(row("报表 PG")).getByText("含密码")).toBeInTheDocument();
    expect(within(row("旧版导出")).getByText("无法解密")).toBeInTheDocument();
    expect(screen.queryByText(/密码存在它自己的钥匙串里/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /^导入 \d+ 个连接$/ }));
    expect(await screen.findByText(/已导入 \d+ 个连接/)).toBeInTheDocument();
    expect(screen.getAllByText("密码已存入钥匙串").length).toBeGreaterThan(0);
    expect(within(screen.getByText("本地笔记").closest("li")!).getByText("无需密码")).toBeInTheDocument();
    const reminder = screen.getByRole("note");
    expect(reminder).toHaveTextContent("请删除");
    expect(reminder).toHaveTextContent(NCX_PATH);
  });

  it("fills the password of a connection imported earlier without one", async () => {
    const earlier: ConnectionConfig = {
      id: "earlier",
      name: "订单库-本机",
      driver: "mysql",
      endpoint: { kind: "network", host: "mysql.example.test", port: 3306, username: "app" },
      savePassword: false,
    };
    const { backend, user } = setup({}, [earlier]);
    await screen.findByRole("radio", { name: /本机 Navicat/ });
    await user.click(screen.getByRole("radio", { name: /Navicat 导出文件/ }));

    expect(await screen.findByText("给「订单库-本机」补上密码，不新建连接")).toBeInTheDocument();
    expect(within(row("订单库")).getByText("补密码")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "选择「订单库」" })).toBeChecked();

    await user.click(screen.getByRole("button", { name: /^导入 \d+ 个连接$/ }));
    expect(await screen.findByText(/为 1 个已有连接补上了密码/)).toBeInTheDocument();
    const after = await backend.listConnections();
    expect(after.filter((c) => c.name === "订单库")).toHaveLength(0);
    expect(after.find((c) => c.id === "earlier")?.savePassword).toBe(true);
  });

  it("keeps the .ncx option usable when Navicat is not installed", async () => {
    const missing: CommandError = { code: "storage", message: "没有找到本机 Navicat 的连接配置", fields: [] };
    const base = createMockBackend();
    const { user } = setup({
      scanImportSource: vi.fn<Backend["scanImportSource"]>(async (source) => {
        if (source.kind === "localNavicat") throw missing;
        return base.scanImportSource(source);
      }),
    });

    const local = await screen.findByRole("radio", { name: /本机 Navicat.*未检测到/ });
    expect(local).toBeDisabled();
    expect(screen.getByText(/没有找到本机 Navicat 的连接配置/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^导入/ })).toBeDisabled();

    await user.click(screen.getByRole("radio", { name: /Navicat 导出文件/ }));
    expect(await screen.findByRole("checkbox", { name: "选择「订单库」" })).toBeChecked();
  });

  it("says plainly that Windows can only import .ncx files", async () => {
    setPlatformForTesting("windows");
    const unavailable: CommandError = {
      code: "unsupported",
      message: "Windows 版 Navicat 的连接和查询存在注册表和它自己的目录里，Basalt 还读不了。请导出 .ncx 再导入。",
      fields: [],
    };
    const base = createMockBackend();
    setup({
      scanImportSource: vi.fn<Backend["scanImportSource"]>(async (source) => {
        if (source.kind === "localNavicat") throw unavailable;
        return base.scanImportSource(source);
      }),
    });
    expect(await screen.findByRole("radio", { name: /本机 Navicat.*Windows 上读不了，请用 \.ncx/ })).toBeDisabled();
    expect(await screen.findByText(/再导入。选择一个来源/)).toBeInTheDocument();
    setPlatformForTesting("mac");
  });

  it("select all checks existing connections too but never unsupported ones", async () => {
    const { user } = setup();
    const all = await screen.findByRole("checkbox", { name: "全选" });
    await screen.findByRole("checkbox", { name: "选择「订单库」" });
    await user.click(all);
    expect(screen.getByRole("checkbox", { name: "选择「本地 MySQL」" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "选择「会话缓存」" })).not.toBeChecked();
    expect(screen.getByRole("button", { name: "导入 4 个连接" })).toBeEnabled();

    await user.click(all);
    expect(screen.getByRole("button", { name: "导入 0 个连接" })).toBeDisabled();
  });

  it("cannot be closed while the import is running", async () => {
    let finish: (report: ImportReport) => void = () => {};
    const { onClose, user } = setup({
      importConnections: vi.fn(() => new Promise<ImportReport>((resolve) => (finish = resolve))),
    });
    await screen.findByRole("checkbox", { name: "选择「订单库」" });
    await user.click(screen.getByRole("button", { name: "导入 3 个连接" }));

    expect(screen.getByRole("button", { name: "取消" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(onClose).not.toHaveBeenCalled();

    finish({
      imported: [{ id: "x", name: "订单库", driver: "mysql", passwordSaved: false }],
      passwordFilled: [],
      skipped: [],
    });
    expect(await screen.findByText("已导入 1 个连接")).toBeInTheDocument();
  });

  it("shows the import error and stays on the preview", async () => {
    const failure: CommandError = { code: "secret", message: "钥匙串操作失败：用户拒绝访问钥匙串", fields: [] };
    const { onClose, user } = setup({
      importConnections: vi.fn<Backend["importConnections"]>(() => {
        throw failure;
      }),
    });
    await screen.findByRole("checkbox", { name: "选择「订单库」" });
    await user.click(screen.getByRole("button", { name: "导入 3 个连接" }));

    expect(await screen.findByText("钥匙串操作失败：用户拒绝访问钥匙串")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "选择「订单库」" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});
