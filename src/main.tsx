import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App, createQueryClient } from "./App";
import { platform } from "./lib/platform";
import { installWebviewGuards } from "./lib/webviewGuards";
import "./styles/global.css";
import "./components/ui/ui.css";
import "./components/layout/layout.css";
import "./components/sidebar/sidebar.css";
import "./components/connection/connection.css";
import "./components/import/import.css";
import "./components/objects/objects.css";
import "./components/monitor/monitor.css";
import "./components/design/design.css";
import "./components/tabs/tabs.css";
import "./components/data/data.css";
import "./components/data/grid.css";
import "./components/query/query.css";
import "./components/query/queryPanels.css";
import "./components/transfer/transfer.css";
import "./components/transfer/transfer-tables.css";
import "./components/tableOps/tableOps.css";
import "./components/inspector/inspector.css";
import "./components/assistant/assistant.css";

// 样式按平台分:Windows 没有红绿灯,标题栏是系统自己的。
document.documentElement.dataset.platform = platform();
// 开发时留着 F5 刷新,打出来的包才拦。
if (platform() === "windows" && import.meta.env.PROD) installWebviewGuards(window);

const root = document.getElementById("root");
if (!root) throw new Error("找不到 #root 挂载点");

createRoot(root).render(
  <StrictMode>
    <App queryClient={createQueryClient()} />
  </StrictMode>,
);
