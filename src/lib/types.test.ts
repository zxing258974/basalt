import { describe, expect, it } from "vitest";
import { errorMessage, isCommandError } from "./types";

describe("isCommandError", () => {
  it("recognises the error object the backend rejects with", () => {
    expect(isCommandError({ code: "timeout", message: "连接超时", fields: [] })).toBe(true);
  });

  it.each([null, "timeout", new Error("boom"), { code: 1, message: "x" }, { code: "x" }])("rejects %o", (value) => {
    expect(isCommandError(value)).toBe(false);
  });
});

describe("errorMessage", () => {
  it.each([
    [{ code: "timeout", message: "连接超时", fields: [] }, "连接超时"],
    [new Error("磁盘已满"), "磁盘已满"],
    ["纯文本", "纯文本"],
    [42, "42"],
  ])("turns %o into a message", (error, message) => {
    expect(errorMessage(error)).toBe(message);
  });
});
