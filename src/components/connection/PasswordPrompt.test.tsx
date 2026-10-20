// 密码框:连接本来就设为保存密码时(比如导入后还没输过密码),默认勾上「记住」。
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { ConnectionConfig } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { PasswordPrompt } from "./PasswordPrompt";

const base: ConnectionConfig = {
  id: "c",
  name: "导入的库",
  driver: "mysql",
  endpoint: { kind: "network", host: "db.example.test", port: 3306, username: "app" },
  savePassword: true,
};

function renderPrompt(connection: ConnectionConfig) {
  setBackendForTesting(createMockBackend([connection]));
  useUi.setState({ passwordPrompt: { connection } });
  render(
    <QueryClientProvider client={createQueryClient()}>
      <PasswordPrompt />
    </QueryClientProvider>,
  );
}

beforeEach(() => useUi.setState({ passwordPrompt: null }));
afterEach(() => setBackendForTesting(null));

describe("password prompt remember default", () => {
  it("is checked when the connection is set to save its password", () => {
    renderPrompt(base);
    expect(screen.getByLabelText("连接成功后把密码保存到钥匙串")).toBeChecked();
  });

  it("stays unchecked for connections that do not save passwords", () => {
    renderPrompt({ ...base, savePassword: false });
    expect(screen.getByLabelText("连接成功后把密码保存到钥匙串")).not.toBeChecked();
  });
});
