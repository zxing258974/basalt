// 记下开着的标签页:查询编辑器里的草稿、还在等连接的上次的标签页,改动后稍等一会儿写进 workspace.json。
// 恢复(启动时读回来)在 hooks/useWorkspace.ts;这里只管「记」,退出前由 useQuitGuard 调 flushWorkspace 写完。
import { create } from "zustand";
import { getBackend } from "../lib/api";
import { errorMessage } from "../lib/types";
import { snapshot, type QueryDraft, type SavedTab } from "../lib/workspace";
import { useTabs } from "./tabs";
import { useUi } from "./ui";

/** 打完字停这么久才写盘:连着打字不会每个键都写一次文件。 */
export const SAVE_DELAY_MS = 500;

interface WorkspaceState {
  drafts: Readonly<Record<string, QueryDraft>>;
  /** 上次的标签页里,还在等连接打开才能放回去的;这期间照样记着,别在记录里把它们弄丢。 */
  waiting: readonly SavedTab[];
  /** 上次的顺序,`waiting` 照它插回去。 */
  order: readonly string[];
  /** 恢复完才开始记:启动时标签栏还是空的,这时记下来会把上次的记录盖掉。 */
  ready: boolean;
  /** 有改动还没写进去。退出前要先写完。 */
  pending: boolean;
}

interface WorkspaceStore extends WorkspaceState {
  setDraft(id: string, patch: Partial<QueryDraft>): void;
  forgetDraft(id: string): void;
  setWaiting(waiting: readonly SavedTab[], order?: readonly string[]): void;
}

const initial: WorkspaceState = { drafts: {}, waiting: [], order: [], ready: false, pending: false };

export const useWorkspace = create<WorkspaceStore>((set) => ({
  ...initial,
  setDraft: (id, patch) =>
    set((s) => ({ drafts: { ...s.drafts, [id]: { sql: "", baseline: "", ...s.drafts[id], ...patch } } })),
  forgetDraft: (id) =>
    set((s) => {
      if (!(id in s.drafts)) return s;
      const { [id]: _gone, ...rest } = s.drafts;
      return { drafts: rest };
    }),
  setWaiting: (waiting, order) => set((s) => ({ waiting, order: order ?? s.order })),
}));

let timer: ReturnType<typeof setTimeout> | null = null;
/** 一次写完再写下一次,免得先发的后到、盖掉新的。 */
let writing: Promise<boolean> = Promise.resolve(true);
let lastError: string | null = null;

function write(): Promise<boolean> {
  writing = writing.then(async () => {
    const { drafts, waiting, order } = useWorkspace.getState();
    const content = JSON.stringify(snapshot(useTabs.getState(), drafts, waiting, order));
    try {
      await (await getBackend()).saveWorkspace(content);
      lastError = null;
      // 写的过程中又有了新改动的话,定时器已经重新排上,还算没写完。
      if (timer === null) useWorkspace.setState({ pending: false });
      return true;
    } catch (error) {
      const message = errorMessage(error);
      // 同样的错只说一次,不每打一个字都弹。
      if (message !== lastError) useUi.getState().notify(`没能记下打开的标签页：${message}`);
      lastError = message;
      return false;
    }
  });
  return writing;
}

/** 有改动了:稍等一会儿再写。 */
export function scheduleSave(): void {
  if (!useWorkspace.getState().ready) return;
  if (timer !== null) clearTimeout(timer);
  timer = setTimeout(() => {
    timer = null;
    void write();
  }, SAVE_DELAY_MS);
  if (!useWorkspace.getState().pending) useWorkspace.setState({ pending: true });
}

/** 马上写完(退出前)。返回有没有写成功;没有要写的直接算成功。 */
export async function flushWorkspace(): Promise<boolean> {
  const { ready, pending } = useWorkspace.getState();
  if (!ready) return true;
  if (timer !== null) {
    clearTimeout(timer);
    timer = null;
  }
  if (!pending) return writing;
  return write();
}

/** 开始记:标签页、草稿、等着的标签页一变就排一次写盘。返回停止记录的函数。 */
export function startRecording(): () => void {
  useWorkspace.setState({ ready: true });
  const stopTabs = useTabs.subscribe((s, prev) => {
    if (s.tabs !== prev.tabs || s.activeId !== prev.activeId) scheduleSave();
  });
  const stopDrafts = useWorkspace.subscribe((s, prev) => {
    if (s.drafts !== prev.drafts || s.waiting !== prev.waiting) scheduleSave();
  });
  return () => {
    stopTabs();
    stopDrafts();
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
}

/** 测试之间复位:定时器、草稿、写盘队列都清掉。 */
export function resetWorkspace(): void {
  if (timer !== null) clearTimeout(timer);
  timer = null;
  writing = Promise.resolve(true);
  lastError = null;
  useWorkspace.setState(initial);
}
