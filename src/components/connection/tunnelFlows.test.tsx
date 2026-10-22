// 打开经 SSH 隧道的连接:密码框里问 SSH 密码 / 私钥口令,第一次连某台服务器先核对指纹。
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { CommandError, ConnectionConfig, SshTunnel } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { renderApp } from "../../test/renderApp";
import { algorithmLabel } from "./HostKeyPrompt";
import { withHostKeyTrust } from "./hostKeyTrust";

const jump: SshTunnel = {
  host: "jump.example.test",
  port: 22,
  username: "deploy",
  auth: "password",
  privateKeyPath: null,
  saveSecret: false,
};

function connection(partial: Partial<ConnectionConfig> = {}): ConnectionConfig {
  return {
    id: "tunneled",
    name: "跳板机后的库",
    driver: "mysql",
    endpoint: { kind: "network", host: "127.0.0.1", port: 3306, username: "root" },
    savePassword: true,
    ssh: jump,
    ...partial,
  };
}

async function openFromTree(backend: Backend) {
  const context = renderApp(backend);
  const node = await screen.findByRole("treeitem", { name: "跳板机后的库" });
  await context.user.dblClick(within(node).getByText("跳板机后的库"));
  return context;
}

async function trustFingerprint(user: ReturnType<typeof renderApp>["user"]) {
  const prompt = within(await screen.findByRole("dialog", { name: "核对 SSH 服务器" }));
  await user.click(prompt.getByRole("button", { name: "信任并连接" }));
}

describe("opening through a tunnel", () => {
  it("asks only for the SSH password when the database password is saved", async () => {
    const backend = createMockBackend([connection()]);
    const remember = vi.spyOn(backend, "rememberSshSecret");
    const rememberDb = vi.spyOn(backend, "rememberPassword");
    const { user } = await openFromTree(backend);

    const prompt = within(await screen.findByRole("dialog", { name: "连接到「跳板机后的库」" }));
    expect(prompt.getByText(/经 deploy@jump.example.test:22/)).toBeInTheDocument();
    expect(prompt.queryByLabelText("密码")).not.toBeInTheDocument();
    await user.type(prompt.getByLabelText("SSH 密码"), "wrong");
    await user.click(prompt.getByRole("button", { name: "连接" }));
    await trustFingerprint(user);
    expect(await prompt.findByText("SSH 用户名或密码不正确")).toBeInTheDocument();

    await user.clear(prompt.getByLabelText("SSH 密码"));
    await user.type(prompt.getByLabelText("SSH 密码"), "right");
    await user.click(prompt.getByRole("button", { name: "连接" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(remember).toHaveBeenCalledWith("tunneled", "right");
    expect(rememberDb).not.toHaveBeenCalled();
    expect((await backend.listConnections())[0]).toMatchObject({ isOpen: true, ssh: { saveSecret: true } });
  });

  it("asks for both passwords when neither is saved and keeps the typed one on retry", async () => {
    const backend = createMockBackend([connection({ savePassword: false })]);
    const { user } = await openFromTree(backend);
    const prompt = within(await screen.findByRole("dialog", { name: "连接到「跳板机后的库」" }));
    await user.type(prompt.getByLabelText("SSH 密码"), "ssh");
    await user.type(prompt.getByLabelText("密码"), "db");
    await user.click(prompt.getByLabelText("连接成功后把密码保存在本机"));
    await user.click(prompt.getByRole("button", { name: "连接" }));
    await trustFingerprint(user);
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect((await backend.listConnections())[0]).toMatchObject({ isOpen: true, savePassword: false });
  });

  it("asks for the key passphrase only when the server side says the key needs one", async () => {
    const keyLogin = connection({
      ssh: { ...jump, auth: "privateKey", privateKeyPath: "/Users/me/.ssh/id_ed25519", saveSecret: false },
    });
    const base = createMockBackend([keyLogin]);
    const needsPassphrase: CommandError = {
      code: "ssh_secret_required",
      message: "私钥 /Users/me/.ssh/id_ed25519 有口令，请输入私钥口令",
      fields: [],
    };
    const backend: Backend = {
      ...base,
      openConnection: vi.fn<Backend["openConnection"]>(async (id, password, sshSecret) => {
        if (!sshSecret) throw needsPassphrase;
        return base.openConnection(id, password, sshSecret);
      }),
    };
    await base.trustHostKey({ host: jump.host, port: jump.port, algorithm: "", fingerprint: "", publicKey: "" });
    const { user } = await openFromTree(backend);

    const prompt = within(await screen.findByRole("dialog", { name: "连接到「跳板机后的库」" }));
    expect(prompt.getByLabelText("私钥口令")).toHaveAccessibleDescription(needsPassphrase.message);
    await user.type(prompt.getByLabelText("私钥口令"), "kp");
    await user.click(prompt.getByRole("button", { name: "连接" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(backend.openConnection).toHaveBeenLastCalledWith("tunneled", undefined, "kp");
  });

  it("leaves the connection closed when the fingerprint is not trusted", async () => {
    const backend = createMockBackend([connection({ ssh: { ...jump, saveSecret: true } })]);
    const { user } = await openFromTree(backend);
    const prompt = within(await screen.findByRole("dialog", { name: "核对 SSH 服务器" }));
    await user.click(prompt.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect((await backend.listConnections())[0]?.isOpen).toBe(false);
  });
});

describe("withHostKeyTrust", () => {
  const unknown: CommandError = {
    code: "host_key_unknown",
    message: "第一次连接",
    fields: [],
    hostKey: { host: "h", port: 22, algorithm: "ssh-ed25519", fingerprint: "SHA256:x", publicKey: "k" },
  };

  it("stops asking after a few unknown keys in a row", async () => {
    const backend = createMockBackend();
    renderApp(backend);
    vi.spyOn(backend, "trustHostKey").mockResolvedValue();
    const action = vi.fn().mockRejectedValue(unknown);
    const run = withHostKeyTrust(action);
    for (let i = 0; i < 3; i++) {
      await waitFor(() => expect(useUi.getState().hostKeyPrompt).not.toBeNull());
      useUi.getState().answerHostKey(true);
    }
    await expect(run).rejects.toMatchObject({ code: "host_key_unknown" });
    expect(action).toHaveBeenCalledTimes(4);
  });

  it("passes other errors through", async () => {
    await expect(withHostKeyTrust(() => Promise.reject(new Error("boom")))).rejects.toThrow("boom");
  });

  it("lets two connections through the same new server share one prompt", async () => {
    const backend = createMockBackend();
    renderApp(backend);
    const trust = vi.spyOn(backend, "trustHostKey").mockResolvedValue();
    // 两个连接各被拒一次(没见过的主机密钥),信任以后各自重试成功。
    const action = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(unknown)
      .mockRejectedValueOnce(unknown)
      .mockResolvedValue("opened");
    const first = withHostKeyTrust(action);
    const second = withHostKeyTrust(action);
    await waitFor(() => expect(useUi.getState().hostKeyPrompt?.resolvers).toHaveLength(2));
    expect(useUi.getState().hostKeyQueue).toHaveLength(0);
    useUi.getState().answerHostKey(true);
    await expect(first).resolves.toBe("opened");
    await expect(second).resolves.toBe("opened");
    expect(trust).toHaveBeenCalled();
    expect(useUi.getState().hostKeyPrompt).toBeNull();
  });

  it("queues prompts for other servers instead of cancelling the one on screen", async () => {
    const other = { ...unknown.hostKey!, host: "other", fingerprint: "SHA256:y" };
    const first = useUi.getState().confirmHostKey(unknown.hostKey!);
    const second = useUi.getState().confirmHostKey(other);
    expect(useUi.getState().hostKeyPrompt?.hostKey.host).toBe("h");
    expect(useUi.getState().hostKeyQueue.map((p) => p.hostKey.host)).toEqual(["other"]);

    useUi.getState().answerHostKey(false);
    await expect(first).resolves.toBe(false);
    expect(useUi.getState().hostKeyPrompt?.hostKey.host).toBe("other");
    // 排队的那个后来又有连接要它:合进同一个。
    const third = useUi.getState().confirmHostKey(other);
    useUi.getState().answerHostKey(true);
    await expect(second).resolves.toBe(true);
    await expect(third).resolves.toBe(true);
    expect(useUi.getState().hostKeyPrompt).toBeNull();
  });

  it("joins a queued prompt for the same server", async () => {
    const other = { ...unknown.hostKey!, host: "other", fingerprint: "SHA256:y" };
    const shown = useUi.getState().confirmHostKey(unknown.hostKey!);
    const queued = useUi.getState().confirmHostKey(other);
    const joined = useUi.getState().confirmHostKey({ ...other, host: "OTHER" });
    expect(useUi.getState().hostKeyQueue).toHaveLength(1);
    expect(useUi.getState().hostKeyQueue[0]?.resolvers).toHaveLength(2);
    useUi.getState().answerHostKey(true);
    useUi.getState().answerHostKey(false);
    await expect(shown).resolves.toBe(true);
    await expect(queued).resolves.toBe(false);
    await expect(joined).resolves.toBe(false);
  });

  it("names key types the way ssh-keygen does", () => {
    expect(algorithmLabel("ssh-ed25519")).toBe("ED25519");
    expect(algorithmLabel("ecdsa-sha2-nistp256")).toBe("ECDSA");
    expect(algorithmLabel("ssh-rsa")).toBe("RSA");
    expect(algorithmLabel("rsa-sha2-512")).toBe("RSA");
  });
});
