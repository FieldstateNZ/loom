import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

// Design-system layers, in cascade order: tokens (custom properties + fonts +
// base reset) then component styles (global `.lm-*` classes).
import "./styles/tokens.css";
import "./styles/components.css";

import { App } from "./App.tsx";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
