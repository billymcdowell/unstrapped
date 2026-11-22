import { useEffect, useState, useSyncExternalStore } from "react";
import { useNav } from "./app-context";

export interface Async<T> { data: T | null; error: string | null; loading: boolean }

/** Fetch on mount / deps change / `data://updated`. Keeps previous data visible while reloading. */
export function useApi<T>(fn: () => Promise<T>, deps: unknown[]): Async<T> {
  const { refreshKey } = useNav();
  const [state, setState] = useState<Async<T>>({ data: null, error: null, loading: true });
  useEffect(() => {
    let alive = true;
    setState((s) => ({ ...s, loading: true }));
    fn().then(
      (data) => alive && setState({ data, error: null, loading: false }),
      (e) => alive && setState((s) => ({ data: s.data, error: String(e?.message ?? e), loading: false })),
    );
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, refreshKey]);
  return state;
}

/** Re-renders when the OS colour scheme flips, so canvas charts can re-read tokens. */
export function useColorScheme(): "light" | "dark" {
  return useSyncExternalStore(
    (cb) => { const m = window.matchMedia("(prefers-color-scheme: dark)"); m.addEventListener("change", cb); return () => m.removeEventListener("change", cb); },
    () => (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"),
  );
}
