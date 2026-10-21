// 转储 SQL 文件与维护表(mock 后端):选内容、选路径、进度能取消、完成后的说法;维护选项按数据库给、
// 结果里列出数据库的消息和前后大小、能再做一项。
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../../lib/mockBackend";
import { MYSQL_CUSTOMERS, SQLITE_NOTES, findRow, openTableMenu } from "../../test/tableMenu";

const CUSTOMERS = { database: "shop", schema: null, table: "customers" };

describe("dump SQL file", { timeout: 15_000 }, () => {
  it("dumps structure and data to the picked file and reports both", async () => {
    const backend = createMockBackend();
    const pick = vi.spyOn(backend, "pickSavePath");
    const dump = vi.spyOn(backend, "dumpTable");
    const { user } = await openTableMenu("转储 SQL 文件…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "转储 SQL 文件" }));
    expect(dialog.getByRole("radio", { name: /^结构和数据/ })).toHaveAttribute("aria-checked", "true");
    expect(dialog.getByText(/DROP TABLE IF EXISTS.*关掉外键检查/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "转储…" }));

    const done = within(await screen.findByRole("dialog", { name: "转储完成" }));
    expect(pick).toHaveBeenCalledWith("customers.sql", { name: "SQL 文件", extensions: ["sql"] });
    expect(dump).toHaveBeenCalledWith(expect.any(String), {
      connectionId: "mock-mysql",
      table: CUSTOMERS,
      withData: true,
      path: "/Users/me/Downloads/customers.sql",
    });
    // 行数单独用等宽字,整句被拆成几段:按整行的文字比较。
    expect(document.querySelector(".transfer-done-title")).toHaveTextContent(/^已转储表结构和 48,000 行数据，文件 /);
    expect(done.getByText("customers.sql")).toBeInTheDocument();
    await user.click(done.getByRole("button", { name: "完成" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("dumps only the structure", async () => {
    const backend = createMockBackend();
    const dump = vi.spyOn(backend, "dumpTable");
    const { user } = await openTableMenu("转储 SQL 文件…", SQLITE_NOTES, backend);
    const dialog = within(screen.getByRole("dialog", { name: "转储 SQL 文件" }));
    expect(dialog.getByText(/触发器放在数据之后/)).toBeInTheDocument();
    await user.click(dialog.getByRole("radio", { name: /^仅结构/ }));
    await user.click(dialog.getByRole("button", { name: "转储…" }));
    const done = within(await screen.findByRole("dialog", { name: "转储完成" }));
    expect(done.getByText(/^已转储表结构，文件/)).toBeInTheDocument();
    expect(dump.mock.calls[0]![1].withData).toBe(false);
  });

  it("stays put when no file is picked and explains cancellation and errors", async () => {
    const backend = createMockBackend();
    const pick = vi.spyOn(backend, "pickSavePath").mockResolvedValueOnce(null);
    const dump = vi
      .spyOn(backend, "dumpTable")
      .mockRejectedValueOnce({ code: "cancelled", message: "已取消", fields: [] })
      .mockRejectedValueOnce({ code: "file", message: "磁盘满了", fields: [] });
    const { user } = await openTableMenu("转储 SQL 文件…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "转储 SQL 文件" }));
    await user.click(dialog.getByRole("button", { name: "转储…" }));
    expect(pick).toHaveBeenCalledTimes(1);
    expect(dump).not.toHaveBeenCalled();

    await user.click(dialog.getByRole("button", { name: "转储…" }));
    expect(await dialog.findByRole("status")).toHaveTextContent("已取消，没有生成文件。");
    await user.click(dialog.getByRole("button", { name: "转储…" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("磁盘满了");
  });

  it("shows progress while writing and can cancel it", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "dumpTable").mockReturnValue(new Promise(() => undefined));
    const cancel = vi.spyOn(backend, "cancelTransfer");
    const { user } = await openTableMenu("转储 SQL 文件…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "转储 SQL 文件" }));
    await user.click(dialog.getByRole("button", { name: "转储…" }));
    expect(await dialog.findByRole("progressbar", { name: "进度" })).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "取消" }));
    expect(cancel).toHaveBeenCalledWith(expect.any(String));
    expect(dialog.getByRole("button", { name: "正在取消…" })).toBeDisabled();
  });
});

describe("maintain table", { timeout: 15_000 }, () => {
  it("lists MySQL's operations and shows the database messages", async () => {
    const backend = createMockBackend();
    const maintain = vi.spyOn(backend, "maintainTable");
    const { user } = await openTableMenu("维护…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "维护表" }));
    expect(dialog.getAllByRole("radio").map((r) => r.querySelector(".transfer-range-label")?.textContent)).toEqual([
      "分析",
      "检查",
      "优化",
      "修复",
    ]);
    await user.click(dialog.getByRole("radio", { name: /^优化OPTIMIZE TABLE.*InnoDB 会重建整张表/ }));
    await user.click(dialog.getByRole("button", { name: "执行" }));

    const result = within(await screen.findByRole("dialog", { name: "优化完成" }));
    expect(maintain).toHaveBeenCalledWith("mock-mysql", CUSTOMERS, "optimize");
    expect(result.getByText("OPTIMIZE TABLE `shop`.`customers`")).toBeInTheDocument();
    const messages = within(result.getByRole("list", { name: "数据库返回的消息" })).getAllByRole("listitem");
    expect(messages.map((m) => m.textContent)).toEqual([
      "提示Table does not support optimize, doing recreate + analyze instead",
      "状态OK",
    ]);
    expect(result.getByText("用时 342 毫秒")).toBeInTheDocument();

    await user.click(result.getByRole("button", { name: "再做一项" }));
    expect(screen.getByRole("dialog", { name: "维护表" })).toBeInTheDocument();
  });

  it("shows how much SQLite's VACUUM saved when there are no messages", async () => {
    const { user } = await openTableMenu("维护…", SQLITE_NOTES);
    const dialog = within(screen.getByRole("dialog", { name: "维护表" }));
    await user.click(dialog.getByRole("radio", { name: /^整理库文件VACUUM.*作用于整个库/ }));
    await user.click(dialog.getByRole("button", { name: "执行" }));
    const result = within(await screen.findByRole("dialog", { name: "整理库文件完成" }));
    expect(result.getByText("执行完了，数据库没有返回消息。")).toBeInTheDocument();
    expect(document.querySelector(".maintenance-meta")).toHaveTextContent("库文件 1.8 MB → 600 KB · 用时 342 毫秒");
    await user.click(result.getByRole("button", { name: "完成" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("can keep running in the background and reports the outcome in the status bar", async () => {
    const backend = createMockBackend();
    let finish: () => void = () => undefined;
    let fail: (error: unknown) => void = () => undefined;
    vi.spyOn(backend, "maintainTable")
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = () =>
              resolve({
                sql: "OPTIMIZE TABLE `shop`.`customers`",
                messages: [{ level: "status", text: "OK" }],
                size: null,
                elapsedMs: 9,
              });
          }),
      )
      .mockImplementationOnce(() => new Promise((_, reject) => (fail = reject)));
    const { user } = await openTableMenu("维护…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "维护表" }));
    await user.click(dialog.getByRole("radio", { name: /^优化/ }));
    await user.click(dialog.getByRole("button", { name: "执行" }));
    await user.click(dialog.getByRole("button", { name: "在后台继续" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    act(() => finish());
    const status = () => within(document.querySelector<HTMLElement>(".statusbar")!).getByRole("alert");
    await waitFor(() => expect(status()).toHaveTextContent("「customers」优化完成：状态 OK。"));

    await user.pointer({ keys: "[MouseRight]", target: await findRow("customers") });
    await user.click(screen.getByRole("menuitem", { name: "维护…" }));
    await user.click(within(screen.getByRole("dialog", { name: "维护表" })).getByRole("button", { name: "执行" }));
    // jsdom 不会把 Esc 变成 <dialog> 的 cancel 事件:直接派发。
    fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    act(() => fail({ code: "database", message: "Lock wait timeout exceeded", fields: [] }));
    await waitFor(() => expect(status()).toHaveTextContent("「customers」分析失败：Lock wait timeout exceeded"));
  });

  it("keeps the choices open when the database refuses", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "maintainTable").mockRejectedValueOnce({
      code: "database",
      message: "Lock wait timeout exceeded",
      fields: [],
    });
    const { user } = await openTableMenu("维护…", MYSQL_CUSTOMERS, backend);
    const dialog = within(screen.getByRole("dialog", { name: "维护表" }));
    await user.click(dialog.getByRole("button", { name: "执行" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("Lock wait timeout exceeded");
    await user.click(dialog.getByRole("radio", { name: /^检查/ }));
    expect(dialog.queryByRole("alert")).not.toBeInTheDocument();
    await waitFor(() => expect(dialog.getByRole("button", { name: "执行" })).toBeEnabled());
  });
});
