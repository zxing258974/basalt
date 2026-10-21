// 表操作的纯逻辑:改名后换成哪个标签页、会丢掉的修改、各家的说法与结果。
import { describe, expect, it } from "vitest";
import { objectsTab, tableDataTab, tableDesignTab } from "../../stores/tabs";
import {
  MAINTENANCE,
  clearChoices,
  clearedNotice,
  formatDuration,
  levelLabel,
  maintenanceNotice,
  pendingWarning,
  pendingWork,
  renamedTab,
  truncateRefusal,
} from "./tableOpsModel";

const users = { database: "FOLIO", schema: "public", table: "users" };

describe("tableOpsModel", () => {
  it("replaces the data and design tabs of a renamed table and leaves the rest", () => {
    const data = tableDataTab("c1", "FOLIO", "public", "users");
    expect(renamedTab(data, "c1", users, "members")).toEqual(tableDataTab("c1", "FOLIO", "public", "members"));
    const design = tableDesignTab("c1", "FOLIO", "public", "users");
    expect(renamedTab(design, "c1", users, "members")).toEqual(tableDesignTab("c1", "FOLIO", "public", "members"));
    expect(renamedTab(objectsTab("c1", "FOLIO", "public"), "c1", users, "members")).toBeNull();
    expect(renamedTab(data, "c2", users, "members")).toBeNull();
  });

  it("counts the work that would be lost and words it", () => {
    const data = tableDataTab("c1", "FOLIO", "public", "users");
    const design = tableDesignTab("c1", "FOLIO", "public", "users");
    const other = tableDataTab("c1", "FOLIO", "public", "orders");
    const state = {
      tabs: [data, design, other],
      pending: { [data.id]: 2, [other.id]: 9 },
      unsaved: { [design.id]: true as const },
    };
    const work = pendingWork(state, "c1", users);
    expect(work).toEqual({ rows: 2, designs: 1 });
    expect(pendingWarning(work, "改名")).toBe(
      "这张表还有 2 行修改没有提交、设计器里的结构修改没有保存，改名后就没有了。",
    );
    expect(pendingWarning({ rows: 0, designs: 0 }, "清空")).toBeNull();
  });

  it("explains clearing per database and words the outcome", () => {
    expect(clearChoices("sqlite")[1]).toMatchObject({ mode: "truncate", sql: "DELETE FROM + 计数归零" });
    expect(clearChoices("postgres")[1]!.detail).toMatch(/RESTART IDENTITY/);
    expect(clearChoices("mariadb")[1]!.detail).toMatch(/不能回滚/);
    expect(clearedNotice(users, "delete", { rows: 12345 })).toBe("已清空表「users」，删除了 12,345 行。");
    expect(clearedNotice(users, "truncate", { rows: null })).toBe("已截断表「users」。");
    expect(truncateRefusal("permission denied")).toBe("permission denied");
    expect(truncateRefusal("cannot truncate a table referenced in a foreign key constraint")).toMatch(
      /\n被别的表的外键引用时不能截断/,
    );
  });

  it("offers each database its own maintenance and labels the results", () => {
    expect(MAINTENANCE.postgres.map((c) => c.operation)).toEqual(["vacuum", "analyze", "reindex"]);
    expect(MAINTENANCE.mariadb).toBe(MAINTENANCE.mysql);
    expect(levelLabel("warning")).toBe("警告");
    expect(levelLabel("Error_Unknown")).toBe("Error_Unknown");
    expect(formatDuration(87)).toBe("87 毫秒");
    expect(formatDuration(12_340)).toBe("12.3 秒");
    const outcome = { sql: "VACUUM", messages: [], size: null, elapsedMs: 1 };
    expect(maintenanceNotice(users, "清理", outcome)).toBe("「users」清理完成：数据库没有返回消息。");
  });
});
