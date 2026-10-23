// 新建 / 编辑连接对话框:选类型、选文件、测试连接、后端字段校验的回显。
import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockBackend } from "../../lib/mockBackend";
import { useUi } from "../../stores/ui";
import { renderApp } from "../../test/renderApp";

async function openCreateDialog() {
  const context = renderApp();
  await context.user.click(within(screen.getByRole("banner")).getByRole("button", { name: "新建连接" }));
  const dialog = within(screen.getByRole("dialog", { name: "新建连接" }));
  return { ...context, dialog };
}

async function openMysqlForm() {
  const context = await openCreateDialog();
  await context.user.click(context.dialog.getByText("MySQL"));
  return context;
}

describe("creating", () => {
  it("creates a Redis connection without a user name and offers no 「preferred」 SSL", async () => {
    const { user, dialog, backend } = await openCreateDialog();
    await user.click(dialog.getByText("Redis"));
    expect(dialog.getByLabelText("端口")).toHaveValue("6379");
    expect(dialog.getByLabelText("用户名（可选）")).toHaveValue("");
    await user.type(dialog.getByLabelText("库号"), "x");
    await user.click(dialog.getByRole("button", { name: "保存" }));
    expect(await dialog.findByText("库号要是 0 或正整数")).toBeInTheDocument();
    await user.clear(dialog.getByLabelText("库号"));
    await user.type(dialog.getByLabelText("库号"), "2");

    await user.click(dialog.getByRole("tab", { name: "SSL" }));
    const modes = within(dialog.getByLabelText("SSL 模式"))
      .getAllByRole("option")
      .map((option) => option.textContent);
    expect(modes).not.toContain("优先");
    expect(dialog.getByLabelText("SSL 模式")).toHaveValue("disabled");

    await user.click(dialog.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    const saved = (await backend.listConnections()).find((c) => c.driver === "redis" && c.id !== "mock-redis");
    expect(saved?.endpoint).toMatchObject({ kind: "network", port: 6379, username: "", database: "2" });
  });

  it("creates a SQLite connection from a picked file and selects it", async () => {
    const { user, dialog, backend } = await openCreateDialog();
    expect(dialog.getByText("选择要连接的数据库类型")).toBeInTheDocument();
    await user.click(dialog.getByText("SQLite"));

    await user.click(dialog.getByRole("button", { name: "浏览…" }));
    expect(await dialog.findByDisplayValue("/Users/me/Documents/new.db")).toBeInTheDocument();
    expect(dialog.getByLabelText("连接名")).toHaveAttribute("placeholder", "new");
    await user.type(dialog.getByLabelText("连接名"), "新笔记");
    await user.click(dialog.getByRole("button", { name: "保存" }));

    const created = await screen.findByRole("treeitem", { name: "新笔记" });
    expect(created).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(await backend.listConnections()).toHaveLength(5);
  });

  it("goes back to the type list", async () => {
    const { user, dialog } = await openMysqlForm();
    await user.click(dialog.getByRole("button", { name: "← 换类型" }));
    expect(dialog.getByText("选择要连接的数据库类型")).toBeInTheDocument();
  });

  it("switches the default port and user along with the type", async () => {
    const { user, dialog } = await openMysqlForm();
    expect(dialog.getByLabelText("端口")).toHaveValue("3306");
    await user.selectOptions(dialog.getByLabelText("类型"), "PostgreSQL");
    expect(dialog.getByLabelText("端口")).toHaveValue("5432");
    expect(dialog.getByLabelText("用户名")).toHaveValue("postgres");
    expect(dialog.getByText("留空时连 postgres 库")).toBeInTheDocument();
  });

  it("closes without saving on cancel", async () => {
    const { user, dialog, backend } = await openMysqlForm();
    await user.click(dialog.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(await backend.listConnections()).toHaveLength(4);
  });
});

describe("testing the connection", () => {
  it("shows the server version and clears it once the form changes", async () => {
    const { user, dialog } = await openMysqlForm();
    await user.click(dialog.getByRole("button", { name: "测试连接" }));
    expect(await dialog.findByRole("status")).toHaveTextContent("连接成功 · MySQL 8.4.3");

    await user.type(dialog.getByLabelText("初始数据库"), "shop");
    expect(dialog.queryByRole("status")).not.toBeInTheDocument();
  });

  it("shows why the test failed", async () => {
    const { user, dialog } = await openMysqlForm();
    await user.clear(dialog.getByLabelText("主机"));
    await user.type(dialog.getByLabelText("主机"), "10.9.9.9");
    await user.click(dialog.getByRole("button", { name: "测试连接" }));
    expect(await dialog.findByRole("status")).toHaveTextContent("连接超时，请检查地址、端口和网络");
  });
});

describe("validation", () => {
  it("marks each field the backend rejected", async () => {
    const { user, dialog } = await openMysqlForm();
    await user.clear(dialog.getByLabelText("主机"));
    await user.clear(dialog.getByLabelText("用户名"));
    await user.click(dialog.getByRole("button", { name: "保存" }));

    expect(await dialog.findByText("请填写主机地址")).toBeInTheDocument();
    expect(dialog.getByText("请填写用户名")).toBeInTheDocument();
    expect(dialog.getByLabelText("主机")).toHaveAttribute("aria-invalid", "true");
    expect(dialog.getByLabelText("端口")).toHaveAttribute("aria-invalid", "false");
  });

  it("shows a save failure that is not about a field", async () => {
    const backend = createMockBackend();
    vi.spyOn(backend, "saveConnection").mockRejectedValue({ code: "io", message: "无法写入配置文件", fields: [] });
    const { user } = renderApp(backend);
    act(() => useUi.getState().openConnectionDialog({ mode: "create", driver: "mysql" }));
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(await screen.findByRole("status")).toHaveTextContent("无法写入配置文件");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});

describe("editing", () => {
  it("keeps the type fixed and hints that the password is already saved", async () => {
    const backend = createMockBackend();
    const { user } = renderApp(backend);
    const [mysql] = await backend.listConnections();
    act(() => useUi.getState().openConnectionDialog({ mode: "edit", connection: mysql! }));

    const dialog = within(screen.getByRole("dialog", { name: "编辑连接" }));
    expect(dialog.getByLabelText("类型")).toBeDisabled();
    expect(dialog.getByLabelText("密码")).toHaveAttribute("placeholder", "已保存在本机");
    expect(dialog.queryByRole("button", { name: "← 换类型" })).not.toBeInTheDocument();

    await user.type(dialog.getByLabelText("密码"), "x");
    expect(dialog.getByLabelText("密码")).toHaveAttribute("placeholder", "");
  });
});
