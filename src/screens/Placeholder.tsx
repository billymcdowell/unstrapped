import type { ReactNode } from "react";

/**
 * Shared placeholder for screens another agent will build.
 * Types, client functions and mocks already exist; see docs/API_CONTRACT.md.
 */
export function Placeholder({ title, plan, uses, children }: { title: string; plan: string; uses: string[]; children?: ReactNode }) {
  return (
    <div className="page">
      <h1 className="page-title">{title}</h1>
      <div className="placeholder">
        <p style={{ fontWeight: 650, color: "var(--text)" }}>Placeholder screen — to be built</p>
        <p style={{ marginTop: 6 }}>{plan}</p>
        <p className="mono faint" style={{ marginTop: 10 }}>Client functions: {uses.join(", ")}</p>
        {children}
      </div>
    </div>
  );
}
