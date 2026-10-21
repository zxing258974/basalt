// 整个应用的交互测试:用 mock 后端渲染 <App />,每次从干净的界面状态开始。
import { useInspector } from "../stores/inspector";
import { useTableOps } from "../stores/tableOps";
import { render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App, createQueryClient } from "../App";
import { setBackendForTesting, type Backend } from "../lib/api";
import { createMockBackend } from "../lib/mockBackend";
import { useTabs } from "../stores/tabs";
import { useUi } from "../stores/ui";

/** store 是模块级单例,不重置的话前一个用例展开的节点、开着的标签会带到下一个。 */
export function resetStores(): void {
  useUi.setState(useUi.getInitialState(), true);
  useTabs.setState(useTabs.getInitialState(), true);
  useInspector.setState(useInspector.getInitialState(), true);
  useTableOps.setState(useTableOps.getInitialState(), true);
}

export function renderApp(backend: Backend = createMockBackend()) {
  resetStores();
  setBackendForTesting(backend);
  const queryClient = createQueryClient();
  const user = userEvent.setup();
  const view = render(<App queryClient={queryClient} />);
  return { ...view, user, backend, queryClient };
}
