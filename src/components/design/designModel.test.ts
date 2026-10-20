import { describe, expect, it } from "vitest";
import type { ColumnDef, TableDesign } from "../../lib/designTypes";
import * as model from "./designModel";

function col(name: string, dataType = "int"): ColumnDef {
  return {
    id: `col:${name}`,
    name,
    dataType,
    nullable: true,
    default: null,
    autoIncrement: false,
    comment: null,
    collation: null,
    onUpdate: null,
    generated: null,
  };
}

function users(): TableDesign {
  return {
    table: { database: "shop", schema: null, table: "users" },
    columns: [col("id"), col("name", "varchar(50)"), col("team_id")],
    primaryKey: ["col:id"],
    primaryKeyName: null,
    indexes: [
      {
        id: "idx:idx_name",
        name: "idx_name",
        columns: [{ column: "col:name", descending: false, length: null }],
        unique: false,
        method: null,
        definition: null,
        isConstraint: false,
      },
    ],
    foreignKeys: [
      {
        id: "fk:fk_team",
        name: "fk_team",
        columns: ["col:team_id", "col:name"],
        refSchema: null,
        refTable: "teams",
        refColumns: ["id", "label"],
        onUpdate: "noAction",
        onDelete: "cascade",
      },
    ],
    checks: [],
    triggers: [],
    options: { engine: "InnoDB", charset: null, collation: null, comment: null, withoutRowid: false, strict: false },
    version: "v1",
  };
}

describe("new tables", () => {
  it("start with an auto-increment primary key in the dialect's integer type", () => {
    const mysql = model.newTableDesign("mariadb", "shop", null);
    expect(mysql.columns[0]).toMatchObject({ name: "id", dataType: "int", autoIncrement: true, nullable: false });
    expect(mysql.primaryKey).toEqual([mysql.columns[0]!.id]);
    expect(mysql.options.engine).toBe("InnoDB");
    expect(model.newTableDesign("postgres", "app", "public").columns[0]!.dataType).toBe("integer");
    const sqlite = model.newTableDesign("sqlite", "main", null);
    expect(sqlite.columns[0]!.dataType).toBe("INTEGER");
    expect(sqlite.options.engine).toBeNull();
    expect(sqlite.columns[0]!.id).not.toMatch(/^col:/);
  });

  it("suggest types, index methods and default types per dialect", () => {
    expect(model.typeSuggestions("mysql")).toContain("varchar(255)");
    expect(model.typeSuggestions("postgres")).toContain("jsonb");
    expect(model.typeSuggestions("sqlite")).toContain("BLOB");
    expect(model.indexMethods("mysql")).toContain("FULLTEXT");
    expect(model.indexMethods("postgres")).toContain("gin");
    expect(model.indexMethods("sqlite")).toEqual([null]);
    expect(model.defaultType("postgres")).toBe("text");
    expect(model.defaultType("sqlite")).toBe("TEXT");
  });
});

describe("column edits", () => {
  it("adds, inserts and updates without touching the original", () => {
    const original = users();
    const [appended, id] = model.addColumn(original, "mysql");
    expect(appended.columns.at(-1)).toMatchObject({ id, name: "", dataType: "varchar(255)" });
    const [inserted] = model.addColumn(original, "mysql", 1);
    expect(inserted.columns[1]!.name).toBe("");
    const renamed = model.updateColumn(original, "col:name", { name: "full_name" });
    expect(renamed.columns[1]!.name).toBe("full_name");
    expect(original.columns[1]!.name).toBe("name");
    expect(model.renameTable(original, "members").table.table).toBe("members");
  });

  it("removing a column drops it from keys, indexes and foreign keys", () => {
    const next = model.removeColumn(users(), "col:name");
    expect(next.columns.map((c) => c.name)).toEqual(["id", "team_id"]);
    expect(next.indexes).toEqual([]);
    expect(next.foreignKeys[0]).toMatchObject({ columns: ["col:team_id"], refColumns: ["id"] });

    const keyless = model.removeColumn(users(), "col:id");
    expect(keyless.primaryKey).toEqual([]);
    const fkGone = model.removeColumn(model.removeColumn(users(), "col:team_id"), "col:name");
    expect(fkGone.foreignKeys).toEqual([]);
  });

  it("moves within bounds and toggles the primary key", () => {
    const original = users();
    expect(model.moveColumn(original, "col:team_id", -1).columns.map((c) => c.name)).toEqual(["id", "team_id", "name"]);
    expect(model.moveColumn(original, "col:id", -1)).toBe(original);
    expect(model.moveColumn(original, "missing", 1)).toBe(original);

    const keyed = model.togglePrimaryKey(original, "col:name");
    expect(keyed.primaryKey).toEqual(["col:id", "col:name"]);
    expect(keyed.columns[1]!.nullable).toBe(false);
    expect(model.togglePrimaryKey(keyed, "col:id").primaryKey).toEqual(["col:name"]);
  });

  it("postgres can only reorder new columns among themselves", () => {
    const original = users();
    const [draft, id] = model.addColumn(original, "postgres");
    const [twice, second] = model.addColumn(draft, "postgres");
    expect(model.canMove("postgres", original, twice, "col:name", 1)).toBe(false);
    expect(model.canMove("postgres", original, twice, id, -1)).toBe(false);
    expect(model.canMove("postgres", original, twice, second, -1)).toBe(true);
    expect(model.canMove("postgres", null, twice, "col:name", 1)).toBe(true);
    expect(model.canMove("mysql", original, twice, "col:name", 1)).toBe(true);
    expect(model.canMove("mysql", original, twice, second, 1)).toBe(false);
  });
});

describe("indexes, foreign keys and options", () => {
  it("adds uniquely named indexes and edits or removes them", () => {
    const once = model.addIndex(model.addIndex(users()));
    expect(once.indexes.map((i) => i.name)).toEqual(["idx_name", "idx_users_2", "idx_users_3"]);
    const nameless = model.addIndex({ ...users(), table: { ...users().table, table: "" }, indexes: [] });
    expect(nameless.indexes[0]!.name).toBe("idx_table_1");
    const unique = model.updateIndex(once, "idx:idx_name", { unique: true });
    expect(unique.indexes[0]!.unique).toBe(true);
    expect(model.removeIndex(unique, "idx:idx_name").indexes).toHaveLength(2);
  });

  it("adds, edits and removes foreign keys and options", () => {
    const added = model.addForeignKey(users());
    const fresh = added.foreignKeys[1]!;
    expect(fresh).toMatchObject({ name: "", columns: [], onDelete: "noAction" });
    const edited = model.updateForeignKey(added, fresh.id, { refTable: "teams" });
    expect(edited.foreignKeys[1]!.refTable).toBe("teams");
    expect(model.removeForeignKey(edited, "fk:fk_team").foreignKeys).toHaveLength(1);
    expect(model.updateOptions(users(), { comment: "用户" }).options.comment).toBe("用户");
  });
});

describe("change tracking", () => {
  it("marks columns as added, edited or unchanged", () => {
    const original = users();
    const [draft, id] = model.addColumn(model.updateColumn(original, "col:name", { comment: "名" }), "mysql");
    expect(
      model.columnStatus(
        original,
        draft.columns.find((c) => c.id === id)!,
      ),
    ).toBe("added");
    expect(model.columnStatus(original, draft.columns[1]!)).toBe("edited");
    expect(model.columnStatus(original, draft.columns[0]!)).toBe("unchanged");
    expect(model.columnStatus(null, original.columns[0]!)).toBe("added");
  });

  it("counts each kind of change once", () => {
    const original = users();
    expect(model.changeCount(original, original)).toBe(0);
    expect(model.changeCount(null, original)).toBe(1);
    let draft = model.updateColumn(original, "col:name", { name: "n" });
    draft = model.removeColumn(draft, "col:team_id");
    draft = model.moveColumn(draft, "col:id", 1);
    draft = model.togglePrimaryKey(draft, "col:name");
    draft = model.addIndex(draft);
    draft = model.updateOptions(model.renameTable(draft, "members"), { comment: "x" });
    // 改名、删字段、换顺序、主键、加索引、删外键(随字段删掉)、选项、表名。
    expect(model.changeCount(original, draft)).toBe(8);
  });
});

describe("rebasing a draft onto a newer structure", () => {
  it("keeps the user's edits and takes everything else from the newer structure", () => {
    const before = users();
    let draft = model.updateColumn(before, "col:name", { comment: "名字" });
    draft = model.removeIndex(draft, "idx:idx_name");
    const [withNew, newId] = model.addColumn(draft, "mysql");
    draft = withNew;

    const after: TableDesign = {
      ...users(),
      version: "v2",
      columns: [col("id"), col("email", "varchar(9)"), col("name", "varchar(50)"), col("team_id", "bigint")],
      indexes: [...users().indexes, { ...users().indexes[0]!, id: "idx:idx_email", name: "idx_email" }],
      options: { ...users().options, comment: "别人改的" },
    };
    const merged = model.rebase(before, after, draft);

    expect(merged.version).toBe("v2");
    expect(merged.columns.map((c) => c.name)).toEqual(["id", "email", "name", "team_id", ""]);
    expect(merged.columns.find((c) => c.name === "name")!.comment).toBe("名字");
    expect(merged.columns.find((c) => c.name === "team_id")!.dataType).toBe("bigint");
    expect(merged.columns.at(-1)!.id).toBe(newId);
    expect(merged.indexes.map((i) => i.name)).toEqual(["idx_email"]);
    expect(merged.options.comment).toBe("别人改的");
  });

  it("re-adds a changed foreign key someone else dropped, and drops an untouched one", () => {
    const before = users();
    const changed = model.updateForeignKey(before, "fk:fk_team", { onDelete: "setNull" });
    const after: TableDesign = { ...users(), version: "v2", foreignKeys: [] };
    const merged = model.rebase(before, after, changed);
    expect(merged.foreignKeys).toHaveLength(1);
    expect(merged.foreignKeys[0]!.id).not.toBe("fk:fk_team");
    expect(model.rebase(before, after, before).foreignKeys).toEqual([]);

    const renamed = model.renameTable(before, "members");
    expect(model.rebase(before, { ...after, table: { ...after.table, table: "x" } }, renamed).table.table).toBe(
      "members",
    );
    const first: TableDesign = { ...after, columns: [col("zero"), ...after.columns] };
    expect(model.rebase(before, first, before).columns[0]!.name).toBe("zero");
  });
});
