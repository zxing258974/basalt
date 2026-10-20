import { describe, expect, it } from "vitest";
import { splitStatements, statementAtCursor, type SqlDialect } from "./splitStatements";

const texts = (source: string, dialect: SqlDialect = "mysql") => splitStatements(source, dialect).map((s) => s.text);

describe("splitStatements", () => {
  it("按分号切分，给出去掉首尾空白后的原文位置", () => {
    const source = "  SELECT 1;\n\nSELECT 2 ;  ";
    const statements = splitStatements(source, "mysql");
    expect(statements).toEqual([
      { text: "SELECT 1", from: 2, to: 10, end: 11 },
      { text: "SELECT 2", from: 13, to: 21, end: 23 },
    ]);
    for (const s of statements) expect(source.slice(s.from, s.to)).toBe(s.text);
  });

  it("最后一条语句可以不带分号", () => {
    expect(splitStatements("SELECT 1;\nSELECT 2", "postgres")).toEqual([
      { text: "SELECT 1", from: 0, to: 8, end: 9 },
      { text: "SELECT 2", from: 10, to: 18, end: 18 },
    ]);
  });

  it("空白和只有分号的片段不算语句", () => {
    expect(texts(";;  ;\n")).toEqual([]);
    expect(texts("")).toEqual([]);
  });

  it("引号里的分号不切", () => {
    expect(texts("SELECT 'a;b', \"c;d\", `e;f`; SELECT 2")).toEqual(["SELECT 'a;b', \"c;d\", `e;f`", "SELECT 2"]);
    expect(texts("SELECT 'it''s; fine'; SELECT 2", "postgres")).toEqual(["SELECT 'it''s; fine'", "SELECT 2"]);
    expect(texts('SELECT "a"";b" FROM t; SELECT 2', "sqlite")).toEqual(['SELECT "a"";b" FROM t', "SELECT 2"]);
  });

  it("MySQL 的反斜杠转义引号，PostgreSQL 的普通字符串不转义", () => {
    expect(texts("SELECT 'a\\'; b'; SELECT 2", "mysql")).toEqual(["SELECT 'a\\'; b'", "SELECT 2"]);
    expect(texts("SELECT 'a\\'; SELECT 2", "postgres")).toEqual(["SELECT 'a\\'", "SELECT 2"]);
    expect(texts("SELECT 'a\\'; SELECT 2", "sqlite")).toEqual(["SELECT 'a\\'", "SELECT 2"]);
  });

  it("PostgreSQL 的 E'' 字符串按反斜杠转义", () => {
    expect(texts("SELECT E'it\\'s; ok', e'\\\\'; SELECT 2", "postgres")).toEqual([
      "SELECT E'it\\'s; ok', e'\\\\'",
      "SELECT 2",
    ]);
    // 标识符结尾的 e 不是 E 前缀。
    expect(texts("SELECT name'x\\'; SELECT 2", "postgres")).toEqual(["SELECT name'x\\'", "SELECT 2"]);
  });

  it("SQLite 的方括号标识符", () => {
    expect(texts("SELECT [a;b] FROM t; SELECT 2", "sqlite")).toEqual(["SELECT [a;b] FROM t", "SELECT 2"]);
  });

  it("注释里的分号不切", () => {
    expect(texts("SELECT 1 -- a;b\n; SELECT /* ; */ 2")).toEqual(["SELECT 1 -- a;b", "SELECT /* ; */ 2"]);
    expect(texts("SELECT 1 # a;b\n; SELECT 2", "mysql")).toEqual(["SELECT 1 # a;b", "SELECT 2"]);
  });

  it("MySQL 的 -- 后面必须跟空白才是注释", () => {
    expect(texts("SELECT 1--;\nSELECT 2", "mysql")).toEqual(["SELECT 1--", "SELECT 2"]);
    expect(texts("SELECT 1--;\nSELECT 2", "postgres")).toEqual(["SELECT 1--;\nSELECT 2"]);
    expect(texts("SELECT 1 --\n; SELECT 2", "mysql")).toEqual(["SELECT 1 --", "SELECT 2"]);
  });

  it("# 只在 MySQL 里是注释", () => {
    expect(texts("SELECT 5 # 3; SELECT 2", "postgres")).toEqual(["SELECT 5 # 3", "SELECT 2"]);
  });

  it("PostgreSQL 的块注释可以嵌套，SQLite 的不行", () => {
    expect(texts("SELECT /* a /* b */ ; */ 1; SELECT 2", "postgres")).toEqual([
      "SELECT /* a /* b */ ; */ 1",
      "SELECT 2",
    ]);
    expect(texts("SELECT /* a /* b */ 1; SELECT 2", "sqlite")).toEqual(["SELECT /* a /* b */ 1", "SELECT 2"]);
  });

  it("只有注释的片段丢掉，语句前的注释随语句一起", () => {
    expect(texts("-- 说明\nSELECT 1;\n/* 结尾 */\n-- 没有语句了")).toEqual(["-- 说明\nSELECT 1"]);
  });

  it("分号后同一行的行注释算上一条的尾巴，不挂到下一条上", () => {
    const statements = splitStatements("SELECT 1; -- 第一条\nSELECT 2;", "mysql");
    expect(statements.map((s) => s.text)).toEqual(["SELECT 1", "SELECT 2"]);
    expect(statements[0]!.end).toBe("SELECT 1; -- 第一条".length);
  });

  it("MySQL 的可执行注释 /*! */ 本身就是语句", () => {
    expect(texts("/*!40101 SET NAMES utf8mb4 */;\nSELECT 1;", "mysql")).toEqual([
      "/*!40101 SET NAMES utf8mb4 */",
      "SELECT 1",
    ]);
  });

  it("PostgreSQL 的 $$ 与带标签的 dollar 引号", () => {
    const source = [
      "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql;",
      "DO $body$ BEGIN PERFORM 1; $x$ ; $x$ END $body$;",
      "SELECT 2",
    ].join("\n");
    expect(texts(source, "postgres")).toEqual([
      "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql",
      "DO $body$ BEGIN PERFORM 1; $x$ ; $x$ END $body$",
      "SELECT 2",
    ]);
  });

  it("$1 参数和标识符里的 $ 不是 dollar 引号", () => {
    expect(texts("SELECT $1; SELECT a$b$c; SELECT 2", "postgres")).toEqual(["SELECT $1", "SELECT a$b$c", "SELECT 2"]);
  });

  it("MySQL 不认 dollar 引号", () => {
    expect(texts("SELECT $$; SELECT 2", "mysql")).toEqual(["SELECT $$", "SELECT 2"]);
  });

  it("MySQL 的 DELIMITER 改分隔符，这一行本身不发给服务器", () => {
    const source = [
      "DELIMITER $$",
      "CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END$$",
      "delimiter ;",
      "CALL p();",
    ].join("\n");
    const statements = splitStatements(source, "mysql");
    expect(statements.map((s) => s.text)).toEqual(["CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END", "CALL p()"]);
    const create = statements[0]!;
    expect(source.slice(create.to, create.end)).toBe("$$");
  });

  it("DELIMITER 之后多字符分隔符里的分号照常是普通字符", () => {
    expect(texts("DELIMITER //\nSELECT 1; SELECT 2//\nSELECT 3//", "mysql")).toEqual([
      "SELECT 1; SELECT 2",
      "SELECT 3",
    ]);
  });

  it("DELIMITER 只在语句开头、只在 MySQL 里生效", () => {
    expect(texts("SELECT 1 AS delimiter; SELECT 2", "mysql")).toEqual(["SELECT 1 AS delimiter", "SELECT 2"]);
    expect(texts("DELIMITER $$\nSELECT 1$$", "postgres")).toEqual(["DELIMITER $$\nSELECT 1$$"]);
  });

  it("MySQL 不写 DELIMITER 也能整体切出 BEGIN … END 的存储过程", () => {
    const source = [
      "CREATE DEFINER=`root`@`%` PROCEDURE p(IN n INT)",
      "BEGIN",
      "  IF n > 0 THEN SELECT 1; END IF;",
      "  label: LOOP LEAVE label; END LOOP;",
      "  SELECT CASE WHEN n THEN 'x' END;",
      "  BEGIN SELECT 2; END;",
      "END;",
      "SELECT 3;",
    ].join("\n");
    const statements = texts(source, "mysql");
    expect(statements).toHaveLength(2);
    expect(statements[0]).toMatch(/^CREATE DEFINER[\s\S]*END$/);
    expect(statements[1]).toBe("SELECT 3");
  });

  it("MariaDB 的 BEGIN NOT ATOMIC 匿名块", () => {
    expect(texts("BEGIN NOT ATOMIC SELECT 1; SELECT 2; END; SELECT 3", "mysql")).toEqual([
      "BEGIN NOT ATOMIC SELECT 1; SELECT 2; END",
      "SELECT 3",
    ]);
  });

  it("限定名里的 end、begin 不是关键字", () => {
    const source = "CREATE PROCEDURE p() BEGIN SELECT t.end FROM t; SELECT 1; END; SELECT 2";
    expect(texts(source, "mysql")).toEqual([
      "CREATE PROCEDURE p() BEGIN SELECT t.end FROM t; SELECT 1; END",
      "SELECT 2",
    ]);
  });

  it("单独的 BEGIN 是开事务，不当成块", () => {
    expect(texts("BEGIN; SELECT 1; COMMIT;", "mysql")).toEqual(["BEGIN", "SELECT 1", "COMMIT"]);
    expect(texts("BEGIN; SELECT 1; END;", "postgres")).toEqual(["BEGIN", "SELECT 1", "END"]);
  });

  it("SQLite 触发器的 BEGIN … END 整体是一条", () => {
    const source =
      "CREATE TEMP TRIGGER t AFTER INSERT ON a BEGIN UPDATE b SET x = CASE WHEN 1 THEN 2 END; INSERT INTO c VALUES (1); END; SELECT 1";
    expect(texts(source, "sqlite")).toEqual([
      "CREATE TEMP TRIGGER t AFTER INSERT ON a BEGIN UPDATE b SET x = CASE WHEN 1 THEN 2 END; INSERT INTO c VALUES (1); END",
      "SELECT 1",
    ]);
  });

  it("PostgreSQL 的 BEGIN ATOMIC 函数体", () => {
    const source = "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END; SELECT 3";
    expect(texts(source, "postgres")).toEqual([
      "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END",
      "SELECT 3",
    ]);
  });

  it("没闭合的引号或注释吞到结尾，交给服务器报错", () => {
    expect(texts("SELECT 'abc; SELECT 2", "mysql")).toEqual(["SELECT 'abc; SELECT 2"]);
    expect(texts("SELECT 1 /* x; SELECT 2", "postgres")).toEqual(["SELECT 1 /* x; SELECT 2"]);
    expect(texts("SELECT $a$ x; SELECT 2", "postgres")).toEqual(["SELECT $a$ x; SELECT 2"]);
  });

  it("中文与表情字符的位置按 UTF-16 下标算，与编辑器一致", () => {
    const source = "SELECT '表情😀';SELECT 2";
    const [first, second] = splitStatements(source, "mysql");
    expect(source.slice(first!.from, first!.to)).toBe("SELECT '表情😀'");
    expect(source.slice(second!.from, second!.to)).toBe("SELECT 2");
  });
});

describe("statementAtCursor", () => {
  const source = "SELECT 1;  -- a\nSELECT 2;\n\n\nSELECT 3";
  const statements = splitStatements(source, "mysql");
  const at = (cursor: number) => statementAtCursor(statements, source, cursor)?.text ?? null;

  it("光标在语句里或紧跟在分号后面", () => {
    expect(at(0)).toBe("SELECT 1");
    expect(at(4)).toBe("SELECT 1");
    expect(at(9)).toBe("SELECT 1");
    expect(at(source.indexOf("SELECT 2") + 3)).toBe("SELECT 2");
  });

  it("同一行分号之后的空白和注释算上一条", () => {
    expect(at(source.indexOf("-- a") + 2)).toBe("SELECT 1");
  });

  it("空行上取下一条，文末取最后一条", () => {
    expect(at(source.indexOf("\n\n") + 1)).toBe("SELECT 3");
    expect(statementAtCursor(splitStatements("SELECT 1;\n\n", "mysql"), "SELECT 1;\n\n", 11)?.text).toBe("SELECT 1");
  });

  it("没有语句时为 null", () => {
    expect(statementAtCursor([], "  ", 1)).toBeNull();
  });
});
