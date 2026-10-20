// 查询标签页的两个小 hook:补全的数据来源、编辑器高度的分隔条。
import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { PointerEvent, ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../../App";
import { setBackendForTesting } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import type { QueryScope } from "../../lib/queryTypes";
import type { DriverKind } from "../../lib/types";
import { useCompletionProvider } from "./useCompletionProvider";
import { EDITOR_DEFAULT, useEditorHeight } from "./useEditorHeight";

afterEach(() => {
  setBackendForTesting(null);
  localStorage.clear();
});

async function provide(connectionId: string, driver: DriverKind, scope: QueryScope) {
  const backend = createMockBackend();
  await backend.openConnection(connectionId, "pw");
  setBackendForTesting(backend);
  const client = createQueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return renderHook(({ scope: current }) => useCompletionProvider(connectionId, driver, current), {
    wrapper,
    initialProps: { scope },
  });
}

describe("useCompletionProvider", () => {
  it("reads MySQL tables, columns and databases from the catalog", async () => {
    const { result } = await provide("mock-mysql", "mysql", { database: "shop", schema: null });
    expect(await result.current.tables(null)).toContain("orders");
    expect(await result.current.tables("mysql")).toEqual(["user"]);
    expect(await result.current.columns("customers", null)).toContain("email");
    expect(await result.current.namespaces()).toContain("shop");
  });

  it("follows the tab's database without being rebuilt", async () => {
    const { result, rerender } = await provide("mock-mysql", "mysql", { database: null, schema: null });
    const first = result.current;
    expect(await first.tables(null)).toEqual([]);
    expect(await first.columns("orders", null)).toEqual([]);
    rerender({ scope: { database: "shop", schema: null } });
    expect(result.current).toBe(first);
    expect(await first.tables(null)).toContain("orders");
  });

  it("uses schemas as the PostgreSQL namespace", async () => {
    const { result } = await provide("mock-pg", "postgres", { database: "folio", schema: "diku_mod_users" });
    expect(await result.current.tables(null)).toEqual(["users"]);
    expect(await result.current.tables("diku_mod_inventory_storage")).toContain("item");
    expect(await result.current.namespaces()).toContain("public");
  });
});

function pointer(clientY: number, parentHeight = 800): PointerEvent<HTMLDivElement> {
  return {
    clientY,
    pointerId: 1,
    currentTarget: { setPointerCapture: () => {}, parentElement: { clientHeight: parentHeight } },
  } as unknown as PointerEvent<HTMLDivElement>;
}

describe("useEditorHeight", () => {
  it("drags within limits, remembers the height and resets on double click", () => {
    const { result } = renderHook(() => useEditorHeight());
    expect(result.current.height).toBe(EDITOR_DEFAULT);
    act(() => result.current.handlers.onPointerDown(pointer(100)));
    act(() => result.current.handlers.onPointerMove(pointer(180)));
    expect(result.current.height).toBe(EDITOR_DEFAULT + 80);
    act(() => result.current.handlers.onPointerMove(pointer(5000)));
    expect(result.current.height).toBe(800 - 120);
    act(() => result.current.handlers.onPointerUp());
    expect(localStorage.getItem("basalt.queryEditorHeight")).toBe("680");
    // 松开之后再移动不再改高度。
    act(() => result.current.handlers.onPointerMove(pointer(0)));
    expect(result.current.height).toBe(680);

    const again = renderHook(() => useEditorHeight());
    expect(again.result.current.height).toBe(680);
    act(() => again.result.current.handlers.onDoubleClick());
    expect(again.result.current.height).toBe(EDITOR_DEFAULT);
  });
});
