import type { ReactNode } from "react";
import type { ScreenId } from "../lib/app-context";

/** Thin 1.5px line icons on a 20px grid, drawn with currentColor. */
function Svg({ children, className = "nav-icon" }: { children: ReactNode; className?: string }) {
  return (
    <svg className={className} viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {children}
    </svg>
  );
}

const PATHS: Record<ScreenId, ReactNode> = {
  home: <><path d="M3.5 9 10 3.5 16.5 9" /><path d="M5 8v8h10V8" /><path d="M8.5 16v-4h3v4" /></>,
  sleep: <path d="M15.5 12.2A6.5 6.5 0 0 1 7.8 4.5a6.5 6.5 0 1 0 7.7 7.7Z" />,
  recovery: <><path d="M10 16.5s-6-3.6-6-8.1A3.4 3.4 0 0 1 10 6.3a3.4 3.4 0 0 1 6 2.1c0 4.5-6 8.1-6 8.1Z" /><path d="M5.5 10.5h2.5l1.2-2 1.6 4 1.2-2h2.5" /></>,
  strain: <><path d="M10 2.8c.6 3-3.8 4.6-3.8 8.6a3.8 3.8 0 0 0 7.6 0c0-1.8-.9-3-1.8-3.9-.2 1.4-1 2.2-2 2.4.8-2.6.6-5.3 0-7.1Z" /></>,
  health: <><path d="M2.5 10h3l1.8-4.5 3.2 9 2.2-6 1.3 1.5h3.5" /></>,
  stress: <><path d="M3 7.5c1.7-1.6 3.3-1.6 5 0s3.3 1.6 5 0 2.6-1.3 4 0" /><path d="M3 12.5c1.7-1.6 3.3-1.6 5 0s3.3 1.6 5 0 2.6-1.3 4 0" /></>,
  trends: <><path d="M3 16.5h14" /><path d="m4 13 4-4.5 3 2.5 5-6" /><circle cx="16" cy="5" r="0.6" fill="currentColor" /></>,
  journal: <><rect x="4.5" y="3" width="11" height="14" rx="1.5" /><path d="M7.5 7h5M7.5 10h5M7.5 13h3" /></>,
  assessments: <><circle cx="10" cy="10" r="6.5" /><circle cx="10" cy="10" r="3.5" /><circle cx="10" cy="10" r="0.6" fill="currentColor" /></>,
  healthspan: <><path d="M6 3h8M6 17h8" /><path d="M6.5 3c0 3.5 3.5 4.5 3.5 7s-3.5 3.5-3.5 7M13.5 3c0 3.5-3.5 4.5-3.5 7s3.5 3.5 3.5 7" /></>,
  data: <><ellipse cx="10" cy="5" rx="5.5" ry="2" /><path d="M4.5 5v10c0 1.1 2.5 2 5.5 2s5.5-.9 5.5-2V5" /><path d="M4.5 10c0 1.1 2.5 2 5.5 2s5.5-.9 5.5-2" /></>,
  strap: <><rect x="6" y="5" width="8" height="10" rx="2" /><path d="M7.5 5V2.5h5V5M7.5 15v2.5h5V15" /></>,
};

export const NavIcon = ({ id }: { id: ScreenId }) => <Svg>{PATHS[id]}</Svg>;

export const ChevronLeft = () => <Svg className=""><path d="m12 4.5-5.5 5.5 5.5 5.5" /></Svg>;
export const ChevronRight = () => <Svg className=""><path d="m8 4.5 5.5 5.5-5.5 5.5" /></Svg>;

/** Brand mark: a thin starburst. */
export function BrandMark() {
  const rays = Array.from({ length: 16 }, (_, i) => (i * Math.PI) / 8);
  return (
    <svg width="18" height="18" viewBox="-10 -10 20 20" aria-hidden="true" stroke="currentColor" strokeWidth={1} strokeLinecap="round">
      {rays.map((a, i) => (
        <line key={i} x1={Math.cos(a) * 2.2} y1={Math.sin(a) * 2.2} x2={Math.cos(a) * (i % 2 ? 6.5 : 9)} y2={Math.sin(a) * (i % 2 ? 6.5 : 9)} />
      ))}
    </svg>
  );
}
