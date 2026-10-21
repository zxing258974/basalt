import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect } from "react";
import { ConnectionDialog } from "./components/connection/ConnectionDialog";
import { DeleteConnectionDialog } from "./components/connection/DeleteConnectionDialog";
import { PasswordPrompt } from "./components/connection/PasswordPrompt";
import { AppShell } from "./components/layout/AppShell";
import { DiscardChangesDialog } from "./components/data/DiscardChangesDialog";
import { TransferDialogs } from "./components/transfer/TransferDialogs";
import { useNewQueryFromSelection } from "./hooks/useNewQueryFromSelection";
import { useQuitGuard } from "./hooks/useQuitGuard";
import { closeTabGuarded } from "./stores/discard";
import { useTabs } from "./stores/tabs";
import { useUi } from "./stores/ui";

/** 数据库报错重试也不会变好,元数据也不会频繁变化。 */
export function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: 30_000, refetchOnWindowFocus: false },
    },
  });
}

function useGlobalShortcuts() {
  const newQuery = useNewQueryFromSelection();
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.metaKey || event.altKey || event.ctrlKey) return;
      const key = event.key.toLowerCase();
      if (key === "n" && !event.shiftKey) {
        event.preventDefault();
        useUi.getState().openConnectionDialog({ mode: "create", driver: null });
      }
      if (key === "t" && !event.shiftKey) {
        event.preventDefault();
        void newQuery();
      }
      if (key === "w" && !event.shiftKey) {
        event.preventDefault();
        const { activeId } = useTabs.getState();
        if (activeId) closeTabGuarded(activeId);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [newQuery]);
}

/** 快捷键要用到 QueryClient,得挂在 Provider 里面。 */
function GlobalShortcuts() {
  useGlobalShortcuts();
  return null;
}

export function App({ queryClient }: { queryClient: QueryClient }) {
  useQuitGuard();
  return (
    <QueryClientProvider client={queryClient}>
      <GlobalShortcuts />
      <AppShell />
      <ConnectionDialog />
      <PasswordPrompt />
      <DeleteConnectionDialog />
      <DiscardChangesDialog />
      <TransferDialogs />
    </QueryClientProvider>
  );
}
