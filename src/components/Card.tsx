import type { ReactNode } from "react";

export interface CardProps {
  title?: string;
  /** Right-aligned link-style action in the header. */
  action?: { label: string; onClick: () => void };
  /** Makes the whole card a button. */
  onClick?: () => void;
  className?: string;
  children: ReactNode;
}

/** Slightly solid panel that sits on the glass; use behind every dial and chart. */
export function Card({ title, action, onClick, className = "", children }: CardProps) {
  const head = (title || action) && (
    <div className="card-head">
      {title ? <h3 className="card-title">{title}</h3> : <span />}
      {action && (
        <button className="card-action" onClick={(e) => { e.stopPropagation(); action.onClick(); }}>
          {action.label}
        </button>
      )}
    </div>
  );
  if (onClick) {
    return (
      <button className={`card clickable ${className}`} onClick={onClick}>
        {head}
        {children}
      </button>
    );
  }
  return (
    <section className={`card ${className}`}>
      {head}
      {children}
    </section>
  );
}
