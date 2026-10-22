// 连接对话框的「SSH」「SSL」两页:填写、测试连接时核对指纹、出错时切到出错的那一页。
import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { createMockBackend } from "../../lib/mockBackend";
import type { ConnectionConfig } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { renderApp } from "../../test/renderApp";

/** 页签的名字后面可能跟着读屏用的「（已设置）」「（有错误）」。 */
const tab = (name: string) => ({ name: new RegExp(`^${name}`) });

const tunneled: ConnectionConfig = {
  id: "tunneled",
  name: "跳板机后的库",
  driver: "postgres",
  endpoint: { kind: "network", host: "127.0.0.1", port: 5432, username: "app" },
  savePassword: true,
  ssh: {
    host: "jump.example.test",
    port: 2222,
    username: "deploy",
    auth: "privateKey",
    privateKeyPath: "/Users/me/.ssh/id_ed25519",
    saveSecret: true,
  },
  ssl: { mode: "verifyCa", caPath: "/Users/me/certs/ca.pem" },
};

async function openForm(driver: "mysql" | "sqlite" = "mysql") {
  const context = renderApp();
  act(() => useUi.getState().openConnectionDialog({ mode: "create", driver }));
  const dialog = within(await screen.findByRole("dialog", { name: "新建连接" }));
  return { ...context, dialog };
}

async function fillTunnel({ user, dialog }: Awaited<ReturnType<typeof openForm>>) {
  await user.click(dialog.getByRole("tab", tab("SSH")));
  await user.click(dialog.getByLabelText("经 SSH 隧道连接"));
  await user.type(dialog.getByLabelText("SSH 主机"), "jump.example.test");
  await user.type(dialog.getByLabelText("SSH 用户名"), "deploy");
}

describe("pages", () => {
  it("offers SSH and SSL pages for network databases only", async () => {
    const mysql = await openForm("mysql");
    const tabs = mysql.dialog.getAllByRole("tab").map((element) => element.textContent);
    expect(tabs).toEqual(["常规", "SSH", "SSL"]);
    expect(mysql.dialog.getByRole("tab", tab("常规"))).toHaveAttribute("aria-selected", "true");
    mysql.unmount();

    const sqlite = await openForm("sqlite");
    expect(sqlite.dialog.queryByRole("tablist")).not.toBeInTheDocument();
  });

  it("switches pages with the arrow keys and keeps what was typed", async () => {
    const { user, dialog } = await openForm();
    await user.type(dialog.getByLabelText("连接名"), "订单库");
    dialog.getByRole("tab", tab("常规")).focus();
    await user.keyboard("{ArrowRight}");
    expect(dialog.getByRole("tab", tab("SSH"))).toHaveFocus();
    expect(dialog.getByRole("tabpanel")).toHaveAttribute("aria-labelledby", dialog.getByRole("tab", tab("SSH")).id);
    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(dialog.getByRole("tab", tab("SSL"))).toHaveAttribute("aria-selected", "true");
    await user.click(dialog.getByRole("tab", tab("常规")));
    expect(dialog.getByLabelText("连接名")).toHaveValue("订单库");
  });

  it("keeps tunnel fields disabled until the tunnel is switched on", async () => {
    const { user, dialog } = await openForm();
    await user.click(dialog.getByRole("tab", tab("SSH")));
    expect(dialog.getByLabelText("SSH 主机")).toBeDisabled();
    expect(dialog.getByLabelText("SSH 端口")).toHaveValue("22");
    expect(dialog.getByLabelText("把 SSH 密码保存到钥匙串")).toBeDisabled();
    await user.click(dialog.getByLabelText("经 SSH 隧道连接"));
    expect(dialog.getByLabelText("SSH 主机")).toBeEnabled();
    // 页签上的小圆点说明这一页开着。
    expect(dialog.getByRole("tab", tab("SSH")).querySelector(".seg-tab-mark.is-on")).not.toBeNull();
    // 读屏读得到:「SSH（已设置）」。
    expect(dialog.getByRole("tab", { name: "SSH（已设置）" })).toBeInTheDocument();
  });
});

describe("SSH", () => {
  it("asks to check the fingerprint on the first test and not again after trusting it", async () => {
    const context = await openForm();
    const { user, dialog } = context;
    await fillTunnel(context);
    await user.type(dialog.getByLabelText("SSH 密码"), "ssh-pass");
    await user.click(dialog.getByRole("button", { name: "测试连接" }));

    const prompt = within(await screen.findByRole("dialog", { name: "核对 SSH 服务器" }));
    expect(prompt.getByText("jump.example.test:22")).toBeInTheDocument();
    // 读屏按「指纹」→ 指纹值来读:dt 紧跟着 dd。
    const term = prompt.getByText("指纹");
    expect(term.tagName).toBe("DT");
    expect(term.nextElementSibling).toHaveTextContent(/^SHA256:/);
    expect(prompt.getByText(/^SHA256:/)).toHaveTextContent(/^SHA256:[A-Za-z0-9+/]{43}$/);
    expect(prompt.getByText("ED25519")).toBeInTheDocument();
    expect(prompt.getByText("ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub")).toBeInTheDocument();
    await user.click(prompt.getByRole("button", { name: "信任并连接" }));

    expect(await dialog.findByRole("status")).toHaveTextContent("连接成功 · MySQL 8.4.3");
    expect(screen.queryByRole("dialog", { name: "核对 SSH 服务器" })).not.toBeInTheDocument();

    await user.click(dialog.getByRole("button", { name: "测试连接" }));
    expect(await dialog.findByRole("status")).toHaveTextContent("连接成功");
    expect(screen.queryByRole("dialog", { name: "核对 SSH 服务器" })).not.toBeInTheDocument();
  });

  it("does nothing when the fingerprint is not trusted", async () => {
    const context = await openForm();
    const { user, dialog } = context;
    await fillTunnel(context);
    await user.type(dialog.getByLabelText("SSH 密码"), "ssh-pass");
    await user.click(dialog.getByRole("button", { name: "测试连接" }));
    await user.click(
      within(await screen.findByRole("dialog", { name: "核对 SSH 服务器" })).getByRole("button", { name: "取消" }),
    );
    expect(screen.queryByRole("dialog", { name: "核对 SSH 服务器" })).not.toBeInTheDocument();
    expect(dialog.queryByRole("status")).not.toBeInTheDocument();
    expect(dialog.getByRole("button", { name: "测试连接" })).toBeEnabled();
  });

  it("says when the SSH password is missing or wrong", async () => {
    const context = await openForm();
    const { user, dialog, backend } = context;
    await backend.trustHostKey({
      host: "jump.example.test",
      port: 22,
      algorithm: "ssh-ed25519",
      fingerprint: "",
      publicKey: "",
    });
    await fillTunnel(context);
    await user.click(dialog.getByRole("button", { name: "测试连接" }));
    expect(await dialog.findByRole("status")).toHaveTextContent("需要 deploy@jump.example.test 的 SSH 密码");
    await user.type(dialog.getByLabelText("SSH 密码"), "wrong");
    await user.click(dialog.getByRole("button", { name: "测试连接" }));
    expect(await dialog.findByRole("status")).toHaveTextContent("SSH 用户名或密码不正确");
  });

  it("turns to the page with the first error when saving fails", async () => {
    const { user, dialog } = await openForm();
    await user.click(dialog.getByRole("tab", tab("SSH")));
    await user.click(dialog.getByLabelText("经 SSH 隧道连接"));
    await user.click(dialog.getByRole("tab", tab("常规")));
    await user.click(dialog.getByRole("button", { name: "保存" }));

    expect(await dialog.findByText("请填写 SSH 主机地址")).toBeInTheDocument();
    expect(dialog.getByRole("tab", tab("SSH"))).toHaveAttribute("aria-selected", "true");
    expect(dialog.getByRole("tab", tab("SSH")).querySelector(".seg-tab-mark.is-error")).not.toBeNull();
    expect(dialog.getByRole("tab", { name: "SSH（有错误）" })).toBeInTheDocument();
    expect(dialog.getByText("请填写 SSH 用户名")).toBeInTheDocument();
  });

  it("logs in with a private key picked from ~/.ssh and saves the tunnel", async () => {
    const context = await openForm();
    const { user, dialog, backend } = context;
    await fillTunnel(context);
    await user.selectOptions(dialog.getByLabelText("登录方式"), "私钥");
    expect(dialog.getByLabelText("私钥口令")).toHaveAttribute("placeholder", "没有口令就留空");
    await user.click(dialog.getByRole("button", { name: "选择私钥文件" }));
    expect(await dialog.findByDisplayValue("/Users/me/.ssh/id_ed25519")).toBeInTheDocument();
    expect(dialog.getByLabelText("把私钥口令保存到钥匙串")).toBeChecked();
    await user.click(dialog.getByRole("button", { name: "保存" }));

    await screen.findByRole("treeitem", { name: "本地 MySQL" });
    const saved = (await backend.listConnections()).at(-1)!;
    expect(saved.ssh).toMatchObject({
      host: "jump.example.test",
      port: 22,
      username: "deploy",
      auth: "privateKey",
      privateKeyPath: "/Users/me/.ssh/id_ed25519",
    });
  });

  it("shows a saved tunnel when editing", async () => {
    const backend = createMockBackend([tunneled]);
    const { user } = renderApp(backend);
    act(() => useUi.getState().openConnectionDialog({ mode: "edit", connection: tunneled }));
    const dialog = within(await screen.findByRole("dialog", { name: "编辑连接" }));
    expect(dialog.getByLabelText("主机")).toHaveAccessibleDescription("从 SSH 服务器那边看的地址");
    await user.click(dialog.getByRole("tab", tab("SSH")));
    expect(dialog.getByLabelText("经 SSH 隧道连接")).toBeChecked();
    expect(dialog.getByLabelText("SSH 端口")).toHaveValue("2222");
    expect(dialog.getByLabelText("私钥口令")).toHaveAttribute("placeholder", "已保存在钥匙串");
    // 换了登录方式,存着的是密码还是口令说不准:框清空。
    await user.selectOptions(dialog.getByLabelText("登录方式"), "密码");
    expect(dialog.getByLabelText("SSH 密码")).toHaveAttribute("placeholder", "");
  });

  it("does not reuse a saved key passphrase as the SSH password after switching the login", async () => {
    const backend = createMockBackend([tunneled]);
    await backend.saveConnection({ ...tunneled, id: "tunneled", sshSecret: "passphrase" });
    await backend.trustHostKey({
      host: "jump.example.test",
      port: 2222,
      algorithm: "",
      fingerprint: "",
      publicKey: "",
    });
    const { user } = renderApp(backend);
    act(() => useUi.getState().openConnectionDialog({ mode: "edit", connection: tunneled }));
    const dialog = within(await screen.findByRole("dialog", { name: "编辑连接" }));
    await user.click(dialog.getByRole("tab", tab("SSH")));
    await user.selectOptions(dialog.getByLabelText("登录方式"), "密码");
    await user.click(dialog.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await expect(backend.openConnection("tunneled")).rejects.toMatchObject({ code: "ssh_secret_required" });
  });
});

describe("SSL", () => {
  it("describes each mode and picks certificate files", async () => {
    const { user, dialog, backend } = await openForm();
    await user.click(dialog.getByRole("tab", tab("SSL")));
    expect(dialog.getByLabelText("SSL 模式")).toHaveValue("preferred");
    expect(dialog.getByText(/没设置过 SSL 的连接都是这样/)).toBeInTheDocument();
    // 「优先」不校验证书:CA 证书框用不上,灰掉并说明。
    expect(dialog.getByLabelText("CA 证书")).toBeDisabled();
    expect(dialog.getByLabelText("CA 证书")).toHaveAccessibleDescription(/用不到 CA 证书/);
    await user.selectOptions(dialog.getByLabelText("SSL 模式"), "必须");
    expect(dialog.getByLabelText("CA 证书")).toBeEnabled();
    expect(dialog.getByLabelText("CA 证书")).toHaveAccessibleDescription(/选了就按它校验服务器证书/);

    await user.selectOptions(dialog.getByLabelText("SSL 模式"), "禁用");
    expect(dialog.getByLabelText("CA 证书")).toBeDisabled();
    await user.selectOptions(dialog.getByLabelText("SSL 模式"), "校验 CA");
    expect(dialog.getByText(/校验服务器证书由所选的 CA 签发/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "选择 CA 证书" }));
    await user.click(dialog.getByRole("button", { name: "选择客户端证书" }));
    await user.click(dialog.getByRole("button", { name: "选择客户端私钥" }));
    expect(await dialog.findByDisplayValue("/Users/me/certs/client.key")).toBeInTheDocument();
    expect(dialog.getByLabelText("CA 证书")).toHaveValue("/Users/me/certs/ca.pem");

    await user.click(dialog.getByRole("button", { name: "保存" }));
    await screen.findByRole("treeitem", { name: "本地 MySQL" });
    const saved = (await backend.listConnections()).at(-1)!;
    expect(saved.ssl).toEqual({
      mode: "verifyCa",
      caPath: "/Users/me/certs/ca.pem",
      clientCertPath: "/Users/me/certs/ca.pem",
      clientKeyPath: "/Users/me/certs/client.key",
    });
  });

  it("does not offer the host name check through a tunnel", async () => {
    const context = await openForm();
    const { user, dialog } = context;
    await fillTunnel(context);
    await user.click(dialog.getByRole("tab", tab("SSL")));
    const option = within(dialog.getByLabelText("SSL 模式")).getByRole("option", {
      name: "校验主机名（经 SSH 隧道时不能用）",
    });
    expect(option).toBeDisabled();
  });

  it("asks for the client certificate and key together", async () => {
    const { user, dialog } = await openForm();
    await user.click(dialog.getByRole("tab", tab("SSL")));
    await user.type(dialog.getByLabelText("客户端证书"), "/Users/me/certs/client.crt");
    await user.click(dialog.getByRole("tab", tab("常规")));
    await user.click(dialog.getByRole("button", { name: "保存" }));
    expect(await dialog.findByText("客户端证书和私钥要一起填")).toBeInTheDocument();
    expect(dialog.getByRole("tab", tab("SSL"))).toHaveAttribute("aria-selected", "true");
  });
});
