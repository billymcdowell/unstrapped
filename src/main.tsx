import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initGlass } from "./lib/glass";
import "@fontsource-variable/urbanist/wght.css";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/app.css";

// Solid background until we know whether glass is available (no flash of see-through window).
document.documentElement.dataset.glass = "none";
initGlass();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
