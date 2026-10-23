import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../App";
import { setBackendForTesting, type Backend } from "../lib/api";
import { createMockBackend } from "../lib/mockBackend";
import { catalogKeys, useConnections, useDatabases, useSchemas, useServerInfo, useTables } from "./useCatalog";

function setup() {
  const backend = createMockBackend();
  setBackendForTesting(backend);
  const queryClient = createQueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { backend, queryClient, wrapper };
}

/** 和 useConnectionActions().open 一样:打开后把服务器版本写进缓存。 */
async function openConnection(backend: Backend, queryClient: QueryClient, id: string) {
  queryClient.setQueryData(catalogKeys.server(id), await backend.openConnection(id, "pw"));
}

describe("catalog queries", () => {
  it("nests every key under its connection so one invalidation clears it", () => {
    const prefix = catalogKeys.connection("c1");
    for (const key of [
      catalogKeys.server("c1"),
      catalogKeys.databases("c1"),
      catalogKeys.schemas("c1", "folio"),
      catalogKeys.tables("c1", "folio", null),
    ]) {
      expect(key.slice(0, prefix.length)).toEqual(prefix);
    }
    expect(catalogKeys.tables("c1", "shop", null)).toEqual(["catalog", "c1", "tables", "shop", ""]);
  });

  it("loads the connection list", async () => {
    const { wrapper } = setup();
    const { result } = renderHook(() => useConnections(), { wrapper });
    await waitFor(() => expect(result.current.data).toHaveLength(4));
  });

  it("reads the server version only from the cache filled on open", async () => {
    const { wrapper, backend, queryClient } = setup();
    const { result, rerender } = renderHook(({ id }) => useServerInfo(id), {
      wrapper,
      initialProps: { id: null as string | null },
    });
    expect(result.current.data).toBeUndefined();

    await openConnection(backend, queryClient, "mock-mysql");
    rerender({ id: "mock-mysql" });
    expect(result.current.data).toEqual({ product: "MySQL", version: "8.4.3" });
  });

  it("does not ask for databases until enabled", async () => {
    const { wrapper, backend, queryClient } = setup();
    const listDatabases = vi.spyOn(backend, "listDatabases");
    const { result, rerender } = renderHook(({ enabled }) => useDatabases("mock-mysql", enabled), {
      wrapper,
      initialProps: { enabled: false },
    });
    expect(listDatabases).not.toHaveBeenCalled();

    await openConnection(backend, queryClient, "mock-mysql");
    rerender({ enabled: true });
    await waitFor(() => expect(result.current.data).toEqual(["information_schema", "mysql", "shop"]));
  });

  it("loads schemas and the tables of one schema", async () => {
    const { wrapper, backend, queryClient } = setup();
    await openConnection(backend, queryClient, "mock-pg");
    const schemas = renderHook(() => useSchemas("mock-pg", "folio", true), { wrapper });
    await waitFor(() => expect(schemas.result.current.data).toContain("public"));

    const tables = renderHook(() => useTables("mock-pg", "folio", "diku_mod_users", true), { wrapper });
    await waitFor(() => expect(tables.result.current.data?.map((t) => t.name)).toEqual(["users"]));
  });
});
