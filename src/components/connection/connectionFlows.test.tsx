// 连接相关的交互流程:密码框重试与「记住密码」时机、编辑已打开连接后的清理。
import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting, type Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { CommandError, ConnectionConfig } from "../../lib/types";
import { objectsTab, useTabs } from "../../stores/tabs";
import { useUi } from "../../stores/ui";
import { ConnectionDialog } from "./ConnectionDialog";
import { PasswordPrompt } from "./PasswordPrompt";

const remote: ConnectionConfig = {
  id: "remote",
  name: "远程库",
  driver: "postgres",
  endpoint: { kind: "network", host: "10.0.0.5", port: 5432, username: "app" },
  savePassword: false,
};

function renderWithClient(ui: React.ReactNode) {
  const client = createQueryClient();
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

beforeEach(() => {
  useUi.setState({ passwordPrompt: null, connectionDialog: { mode: "closed" }, notice: null });
  useTabs.setState({ tabs: [], activeId: null });
});

afterEach(() => setBackendForTesting(null));

describe("password prompt", () => {
  it("shows the error after a wrong password and remembers only after a successful open", async () => {
    const base = createMockBackend([remote]);
    const authError: CommandError = { code: "authentication", message: "用户名或密码不正确：密码错误", fields: [] };
    const backend: Backend = {
      ...base,
      openConnection: vi.fn<Backend["openConnection"]>(async (id, password) => {
        if (password !== "right") throw authError;
        return base.openConnection(id, "right");
      }),
      rememberPassword: vi.fn(async () => {}),
    };
    setBackendForTesting(backend);
    useUi.setState({ passwordPrompt: { connection: remote } });
    const user = userEvent.setup();
    renderWithClient(<PasswordPrompt />);

    await user.type(screen.getByLabelText("密码"), "wrong");
    expect(screen.getByLabelText("连接成功后把密码保存到钥匙串")).toBeChecked();
    await user.click(screen.getByRole("button", { name: "连接" }));

    expect(await screen.findByText("用户名或密码不正确：密码错误")).toBeInTheDocument();
    expect(backend.rememberPassword).not.toHaveBeenCalled();

    await user.clear(screen.getByLabelText("密码"));
    await user.type(screen.getByLabelText("密码"), "right");
    await user.click(screen.getByRole("button", { name: "连接" }));

    await waitFor(() => expect(useUi.getState().passwordPrompt).toBeNull());
    await waitFor(() => expect(backend.rememberPassword).toHaveBeenCalledWith("remote", "right"));
  });

  it("stays connected but says so when saving the password fails", async () => {
    const backend = createMockBackend([remote]);
    vi.spyOn(backend, "rememberPassword").mockRejectedValue(new Error("钥匙串已锁定"));
    setBackendForTesting(backend);
    useUi.setState({ passwordPrompt: { connection: remote } });
    const user = userEvent.setup();
    renderWithClient(<PasswordPrompt />);

    await user.type(screen.getByLabelText("密码"), "pw");
    // 「记住」默认勾选(导入的连接只需输一次密码),这里不用再点。
    expect(screen.getByLabelText("连接成功后把密码保存到钥匙串")).toBeChecked();
    await user.click(screen.getByRole("button", { name: "连接" }));

    await waitFor(() => expect(useUi.getState().notice?.message).toBe("已连接，但保存密码失败：钥匙串已锁定"));
    expect((await backend.listConnections())[0]?.isOpen).toBe(true);
  });

  it("shows failures other than a wrong password inside the prompt", async () => {
    const backend = createMockBackend([remote]);
    vi.spyOn(backend, "openConnection").mockRejectedValue(new Error("连接超时"));
    setBackendForTesting(backend);
    useUi.setState({ passwordPrompt: { connection: remote } });
    const user = userEvent.setup();
    renderWithClient(<PasswordPrompt />);

    await user.type(screen.getByLabelText("密码"), "pw{Enter}");
    expect(await screen.findByText("连接超时")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(useUi.getState().passwordPrompt).toBeNull();
  });

  it("closes on Escape", () => {
    setBackendForTesting(createMockBackend([remote]));
    useUi.setState({ passwordPrompt: { connection: remote } });
    renderWithClient(<PasswordPrompt />);
    fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(useUi.getState().passwordPrompt).toBeNull();
  });
});

describe("editing an open connection", () => {
  const local: ConnectionConfig = {
    id: "local",
    name: "本地",
    driver: "mysql",
    endpoint: { kind: "network", host: "127.0.0.1", port: 3306, username: "root" },
    savePassword: true,
  };

  async function openAndEdit() {
    const backend = createMockBackend([local]);
    setBackendForTesting(backend);
    await backend.openConnection("local");
    useTabs.getState().open(objectsTab("local", "shop", null));
    useUi.setState({ connectionDialog: { mode: "edit", connection: local } });
    const user = userEvent.setup();
    renderWithClient(<ConnectionDialog />);
    return { user, backend };
  }

  it("keeps the tabs when only the name changes", async () => {
    const { user } = await openAndEdit();
    const name = screen.getByLabelText("连接名");
    await user.clear(name);
    await user.type(name, "本地改名");
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(useUi.getState().connectionDialog.mode).toBe("closed"));
    expect(useTabs.getState().tabs).toHaveLength(1);
  });

  it("closes the tabs when the host changes and the session is dropped", async () => {
    const { user, backend } = await openAndEdit();
    const host = screen.getByLabelText("主机");
    await user.clear(host);
    await user.type(host, "10.0.0.9");
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(useUi.getState().connectionDialog.mode).toBe("closed"));
    expect(useTabs.getState().tabs).toHaveLength(0);
    expect((await backend.listConnections())[0]?.isOpen).toBe(false);
  });
});
