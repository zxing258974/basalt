import { describe, expect, it } from "vitest";
import type { ImportCandidate, ImportReport } from "../../lib/importTypes";
import {
  candidateAddress,
  defaultSelection,
  passwordLabel,
  reportHeadline,
  selectAll,
  selectableIds,
  selectionSummary,
  statusLabel,
  toggleSelection,
  transportNotes,
} from "./importModel";

function candidate(id: string, partial: Partial<ImportCandidate> = {}): ImportCandidate {
  return {
    id,
    name: id,
    group: null,
    sourceType: "MySQL",
    driver: "mysql",
    endpoint: { kind: "network", host: "db.example.test", port: 3306, username: "app" },
    status: { kind: "new" },
    password: "included",
    ssh: null,
    sshSecret: null,
    sslMode: "preferred",
    warnings: [],
    ...partial,
  };
}

const candidates = [
  candidate("new-1"),
  candidate("new-2", { password: "absent" }),
  candidate("dup", { status: { kind: "existing", existingName: "旧库" } }),
  candidate("redis", {
    sourceType: "Redis",
    driver: null,
    endpoint: null,
    status: { kind: "unsupported", reason: "暂不支持 Redis" },
  }),
];

describe("selection", () => {
  it("checks only new connections by default", () => {
    expect([...defaultSelection(candidates)]).toEqual(["new-1", "new-2"]);
  });

  it("never lets unsupported connections be selected", () => {
    expect(selectableIds(candidates)).toEqual(["new-1", "new-2", "dup"]);
    expect(toggleSelection(new Set(), candidates, "redis").has("redis")).toBe(false);
  });

  it("toggles without mutating the previous selection", () => {
    const before = defaultSelection(candidates);
    const after = toggleSelection(before, candidates, "dup");
    expect(after.has("dup")).toBe(true);
    expect(before.has("dup")).toBe(false);
    expect(toggleSelection(after, candidates, "new-1").has("new-1")).toBe(false);
  });

  it("select all covers existing ones too, and clears everything when unchecked", () => {
    expect([...selectAll(candidates, true)]).toEqual(["new-1", "new-2", "dup"]);
    expect(selectAll(candidates, false).size).toBe(0);
  });

  it("summarizes how many selected connections bring a password", () => {
    expect(selectionSummary(candidates, new Set(["new-1", "new-2"]))).toBe("已选 2 个，其中 1 个含密码");
    expect(selectionSummary(candidates, new Set())).toBe("未选择连接");
  });
});

describe("labels", () => {
  it("names each status and password state", () => {
    expect(candidates.map((c) => statusLabel(c.status))).toEqual(["新增", "新增", "已存在", "暂不支持"]);
    expect(passwordLabel("included")).toBe("含密码");
    expect(passwordLabel("absent")).toBe("不含密码");
    expect(passwordLabel("undecryptable")).toBe("无法解密");
  });

  it("shows the address or a dash when unsupported", () => {
    expect(candidateAddress(candidates[0]!)).toBe("app@db.example.test:3306");
    expect(
      candidateAddress(candidate("f", { driver: "sqlite", endpoint: { kind: "file", path: "/data/notes.db" } })),
    ).toBe("notes.db");
    expect(candidateAddress(candidates[3]!)).toBe("—");
  });

  it("headlines the report", () => {
    const report: ImportReport = {
      imported: [
        { id: "a", name: "A", driver: "mysql", passwordSaved: true, tunneled: false, sshSecretSaved: false },
        { id: "b", name: "B", driver: "postgres", passwordSaved: false, tunneled: false, sshSecretSaved: false },
      ],
      passwordFilled: [],
      skipped: [{ name: "C", reason: "暂不支持" }],
    };
    expect(reportHeadline(report)).toBe("已导入 2 个连接，跳过 1 个");
    expect(reportHeadline({ ...report, skipped: [] })).toBe("已导入 2 个连接");
    expect(reportHeadline({ ...report, imported: [] })).toBe("没有导入任何连接，跳过 1 个");
  });

  it("mentions passwords filled into existing connections", () => {
    const filled = [
      { id: "x", name: "cloud-mysql" },
      { id: "y", name: "analytics" },
    ];
    expect(reportHeadline({ imported: [], passwordFilled: filled, skipped: [] })).toBe("为 2 个已有连接补上了密码");
    expect(
      reportHeadline({
        imported: [
          { id: "a", name: "A", driver: "mysql", passwordSaved: true, tunneled: false, sshSecretSaved: false },
        ],
        passwordFilled: filled,
        skipped: [{ name: "C", reason: "暂不支持" }],
      }),
    ).toBe("已导入 1 个连接，为 2 个已有连接补上了密码，跳过 1 个");
  });
});

describe("filling passwords into existing connections", () => {
  const fill = candidate("fill", { status: { kind: "fillPassword", existingId: "c9", existingName: "cloud-mysql" } });

  it("is checked by default and labelled as such", () => {
    expect(defaultSelection([...candidates, fill]).has("fill")).toBe(true);
    expect(statusLabel(fill.status)).toBe("补密码");
  });
});

describe("tunnel and SSL notes", () => {
  const ssh = {
    host: "jump",
    port: 22,
    username: "deploy",
    auth: "privateKey" as const,
    privateKeyPath: "/k",
    saveSecret: false,
  };

  it("describes the tunnel and a non-default SSL mode", () => {
    expect(transportNotes(candidate("a"))).toEqual([]);
    expect(transportNotes(candidate("b", { ssh, sshSecret: "included", sslMode: "required" }))).toEqual([
      "经 deploy@jump:22 的 SSH 隧道，含私钥口令",
      "SSL：必须",
    ]);
    expect(transportNotes(candidate("c", { ssh: { ...ssh, auth: "password" }, sshSecret: "absent" }))).toEqual([
      "经 deploy@jump:22 的 SSH 隧道",
    ]);
  });

  it("says nothing for connections that cannot be imported", () => {
    const unsupported = candidate("d", { ssh, sslMode: "required", status: { kind: "unsupported", reason: "x" } });
    expect(transportNotes(unsupported)).toEqual([]);
  });
});
