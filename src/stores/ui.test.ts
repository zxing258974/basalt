import { beforeEach, describe, expect, it } from "vitest";
import { useUi } from "./ui";

describe("notices", () => {
  beforeEach(() => useUi.setState({ notice: null }));

  it("keeps only the latest notice and gives each one a fresh id", () => {
    useUi.getState().notify("复制失败");
    const first = useUi.getState().notice;
    useUi.getState().notify("关闭失败");
    const second = useUi.getState().notice;
    expect(second?.message).toBe("关闭失败");
    expect(second?.id).not.toBe(first?.id);
  });

  it("dismisses only the notice it was asked about", () => {
    useUi.getState().notify("旧的");
    const stale = useUi.getState().notice!;
    useUi.getState().notify("新的");
    useUi.getState().dismissNotice(stale.id);
    expect(useUi.getState().notice?.message).toBe("新的");
    useUi.getState().dismissNotice(useUi.getState().notice!.id);
    expect(useUi.getState().notice).toBeNull();
  });
});

describe("connection errors", () => {
  it("sets and clears an error without touching others", () => {
    const { setConnectionError } = useUi.getState();
    setConnectionError("a", "连不上");
    setConnectionError("b", "超时");
    setConnectionError("a", null);
    expect(useUi.getState().connectionErrors).toEqual({ b: "超时" });
  });
});
