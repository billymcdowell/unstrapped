/** True when running inside the Tauri webview (false in a plain browser, where mocks are used). */
export const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
