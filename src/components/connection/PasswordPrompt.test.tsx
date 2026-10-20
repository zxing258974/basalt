// 密码框:「记住」默认勾上,连接成功后把密码写进钥匙串,用户只需输一次。
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting, type Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { ConnectionConfig } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { PasswordPrompt } from "./PasswordPrompt";

const base: ConnectionConfig = {
  id: "c",
  name: "导入的库",
  driver: "mysql",
  endpoint: { kind: "network", host: "db.example.test", port: 3306, username: "app" },
  savePassword: false,
};

function renderPrompt(connection: ConnectionConfig) {
  const base = createMockBackend([connection]);
  const backend: Backend = { ...base, rememberPassword: vi.fn(base.rememberPassword) };
  setBackendForTesting(backend);
  useUi.setState({ passwordPrompt: { connection } });
  render(
    <QueryClientProvider client={createQueryClient()}>
      <PasswordPrompt />
    </QueryClientProvider>,
  );
  return backend;
}

beforeEach(() => useUi.setState({ passwordPrompt: null }));
afterEach(() => setBackendForTesting(null));

describe("password prompt remember default", () => {
  it("is checked by default and remembers the password after a successful open", async () => {
    const backend = renderPrompt(base);
    expect(screen.getByLabelText("连接成功后把密码保存到钥匙串")).toBeChecked();

    const user = userEvent.setup();
    await user.type(screen.getByLabelText("密码"), "pw");
    await user.click(screen.getByRole("button", { name: "连接" }));

    await waitFor(() => expect(backend.rememberPassword).toHaveBeenCalledWith("c", "pw"));
    expect((await backend.listConnections())[0]?.savePassword).toBe(true);
  });

  it("does not remember when the user unticks it", async () => {
    const backend = renderPrompt(base);
    const user = userEvent.setup();
    await user.click(screen.getByLabelText("连接成功后把密码保存到钥匙串"));
    await user.type(screen.getByLabelText("密码"), "pw");
    await user.click(screen.getByRole("button", { name: "连接" }));

    await waitFor(() => expect(useUi.getState().passwordPrompt).toBeNull());
    expect(backend.rememberPassword).not.toHaveBeenCalled();
  });
});
