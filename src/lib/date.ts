/** Day/time helpers. Days are local "YYYY-MM-DD" strings; instants are Unix ms. */
import type { Day } from "../api/types";

const pad = (n: number) => String(n).padStart(2, "0");

export function toDay(d: Date): Day {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
export const today = (): Day => toDay(new Date());

export function parseDay(day: Day): Date {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y, m - 1, d);
}
export const dayStartMs = (day: Day): number => parseDay(day).getTime();
export const dayEndMs = (day: Day): number => dayStartMs(addDays(day, 1));

export function addDays(day: Day, n: number): Day {
  const d = parseDay(day);
  d.setDate(d.getDate() + n);
  return toDay(d);
}
export function daysBetween(a: Day, b: Day): number {
  return Math.round((dayStartMs(b) - dayStartMs(a)) / 86_400_000);
}

export function formatDayLong(day: Day): string {
  const t = today();
  if (day === t) return "Today";
  if (day === addDays(t, -1)) return "Yesterday";
  return parseDay(day).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
}
export function formatDayShort(day: Day): string {
  return parseDay(day).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

export function fmtTime(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
/** 485 -> "8h 05m" */
export function fmtDuration(min: number): string {
  const m = Math.round(min);
  const h = Math.floor(m / 60);
  return h > 0 ? `${h}h ${pad(m % 60)}m` : `${m}m`;
}
/** 485 -> "8:05" */
export function fmtHM(min: number): string {
  const m = Math.round(min);
  return `${Math.floor(m / 60)}:${pad(m % 60)}`;
}
