import { describe, expect, it } from "vitest";
import type { ConnectionDraft } from "../types";
import { createMockTunnels, mockHostKey, validateTransport } from "./tunnelMock";

const draft = (partial: Partial<ConnectionDraft>): ConnectionDraft => ({
  name: "n",
  driver: "mysql",
  endpoint: { kind: "network", host: "db", port: 3306, username: "root" },
  savePassword: false,
  ...partial,
});

const ssh = { host: "jump", port: 22, username: "u", auth: "password" as const, saveSecret: false };

describe("tunnel mock", () => {
  it("gives every server a stable fingerprint", () => {
    expect(mockHostKey(ssh)).toEqual(mockHostKey({ ...ssh, host: "JUMP" }));
    expect(mockHostKey(ssh).fingerprint).not.toBe(mockHostKey({ ...ssh, port: 2222 }).fingerprint);
    expect(mockHostKey(ssh).fingerprint).toMatch(/^SHA256:[A-Za-z0-9+/]{43}$/);
  });

  it("validates the same fields as the backend", () => {
    expect(validateTransport(draft({ endpoint: { kind: "file", path: "/a.db" }, ssh }))).toEqual([]);
    const fields = (d: ConnectionDraft) => validateTransport(d).map((e) => e.field);
    expect(fields(draft({ ssh: { ...ssh, host: " ", port: 0, username: "", auth: "privateKey" } }))).toEqual([
      "sshHost",
      "sshPort",
      "sshUsername",
      "sshPrivateKey",
    ]);
    expect(fields(draft({ ssh, ssl: { mode: "verifyIdentity", clientKeyPath: "/k" } }))).toEqual([
      "sslMode",
      "sslCert",
    ]);
    expect(fields(draft({ ssl: { mode: "disabled", clientCertPath: "/c" } }))).toEqual([]);
  });

  it("answers like the backend before and after trusting the server", () => {
    const tunnels = createMockTunnels();
    expect(() => tunnels.check(null, undefined)).not.toThrow();
    expect(() => tunnels.check(ssh, "pw")).toThrow(expect.objectContaining({ code: "host_key_unknown" }));
    tunnels.trust(mockHostKey(ssh));
    expect(() => tunnels.check(ssh, undefined)).toThrow(expect.objectContaining({ code: "ssh_secret_required" }));
    expect(() => tunnels.check(ssh, "wrong")).toThrow(expect.objectContaining({ code: "ssh_authentication" }));
    expect(() => tunnels.check({ ...ssh, auth: "privateKey" }, undefined)).not.toThrow();
  });
});
