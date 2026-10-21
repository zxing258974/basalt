// mock 后端的 SSH 隧道与 SSL:浏览器里没有真的 SSH,只模拟后端会给的几种回应——
// 第一次连接要核对指纹、没给 SSH 密码、SSH 密码不对、表单校验。错误码和文案与 Rust 端一致。
import type { CommandError, ConnectionDraft, FieldError, HostKey, SshTunnel } from "../types";

/** mock 里「不对」的 SSH 密码,用来演示登录失败。 */
export const MOCK_WRONG_SSH_SECRET = "wrong";

function fail(code: string, message: string, extra: Partial<CommandError> = {}): never {
  const error: CommandError = { code, message, fields: [], ...extra };
  throw error;
}

/** 按主机名算出固定的假指纹,同一台「服务器」每次都一样。 */
export function mockHostKey(ssh: Pick<SshTunnel, "host" | "port">): HostKey {
  const host = ssh.host.toLowerCase();
  let state = Array.from(`${host}:${ssh.port}`).reduce((sum, ch) => (sum * 31 + ch.charCodeAt(0)) >>> 0, 7);
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const digest = Array.from({ length: 43 }, () => {
    state = (Math.imul(state, 1103515245) + 12345) >>> 0;
    return alphabet[state >>> 26]!;
  }).join("");
  return {
    host,
    port: ssh.port,
    algorithm: "ssh-ed25519",
    fingerprint: `SHA256:${digest}`,
    publicKey: `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI${digest.slice(0, 32)}`,
  };
}

const address = (key: Pick<HostKey, "host" | "port">) => `${key.host.toLowerCase()}:${key.port}`;

/** 和 core 的 config/transport.rs 一致的那部分校验。 */
export function validateTransport(draft: ConnectionDraft): FieldError[] {
  if (draft.endpoint.kind !== "network") return [];
  const errors: FieldError[] = [];
  const { ssh, ssl } = draft;
  if (ssh) {
    if (!ssh.host.trim()) errors.push({ field: "sshHost", message: "请填写 SSH 主机地址" });
    if (!ssh.port) errors.push({ field: "sshPort", message: "端口范围是 1–65535" });
    if (!ssh.username.trim()) errors.push({ field: "sshUsername", message: "请填写 SSH 用户名" });
    if (ssh.auth === "privateKey" && !ssh.privateKeyPath?.trim()) {
      errors.push({ field: "sshPrivateKey", message: "请选择私钥文件" });
    }
  }
  if (ssl && ssl.mode !== "disabled") {
    if (ssh && ssl.mode === "verifyIdentity") {
      errors.push({
        field: "sslMode",
        message: "经 SSH 隧道连接时没法核对主机名（实际连的是本机的转发端口），请改用「校验 CA」",
      });
    }
    const cert = Boolean(ssl.clientCertPath?.trim());
    const key = Boolean(ssl.clientKeyPath?.trim());
    if (cert && !key) errors.push({ field: "sslKey", message: "客户端证书和私钥要一起填" });
    if (key && !cert) errors.push({ field: "sslCert", message: "客户端证书和私钥要一起填" });
  }
  return errors;
}

export function createMockTunnels() {
  const trusted = new Set<string>();
  return {
    trust(hostKey: HostKey) {
      trusted.add(address(hostKey));
    },
    /** 经隧道连接之前:没核对过指纹、缺密码、密码不对时抛出和后端一样的错误。 */
    check(ssh: SshTunnel | null | undefined, secret: string | undefined) {
      if (!ssh) return;
      const hostKey = mockHostKey(ssh);
      if (!trusted.has(address(hostKey))) {
        fail(
          "host_key_unknown",
          `第一次连接 SSH 服务器 ${hostKey.host}:${hostKey.port}，请核对它的主机密钥指纹 ${hostKey.fingerprint}`,
          {
            hostKey,
          },
        );
      }
      if (ssh.auth === "password" && secret === undefined) {
        fail("ssh_secret_required", `需要 ${ssh.username}@${ssh.host} 的 SSH 密码`);
      }
      if (secret === MOCK_WRONG_SSH_SECRET) fail("ssh_authentication", "SSH 用户名或密码不正确");
    },
  };
}

export async function mockCredentialFile(kind: "sshKey" | "certificate" | "key"): Promise<string> {
  await Promise.resolve();
  switch (kind) {
    case "sshKey":
      return "/Users/me/.ssh/id_ed25519";
    case "certificate":
      return "/Users/me/certs/ca.pem";
    case "key":
      return "/Users/me/certs/client.key";
  }
}
