import React from "react";
import ReactDOM from "react-dom/client";
import "./styles.css";
import App from "./App";

// Outside the tray app (t3boat-server, or vite on its own) the panel sits on a page: framed.
if (!("__TAURI_INTERNALS__" in window)) document.documentElement.classList.add("web");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
