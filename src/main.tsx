import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App, createQueryClient } from "./App";
import "./styles/global.css";
import "./components/ui/ui.css";
import "./components/layout/layout.css";
import "./components/sidebar/sidebar.css";
import "./components/connection/connection.css";
import "./components/import/import.css";
import "./components/objects/objects.css";
import "./components/design/design.css";
import "./components/tabs/tabs.css";
import "./components/data/data.css";
import "./components/data/grid.css";
import "./components/query/query.css";
import "./components/query/queryPanels.css";
import "./components/transfer/transfer.css";
import "./components/transfer/transfer-tables.css";

const root = document.getElementById("root");
if (!root) throw new Error("找不到 #root 挂载点");

createRoot(root).render(
  <StrictMode>
    <App queryClient={createQueryClient()} />
  </StrictMode>,
);
