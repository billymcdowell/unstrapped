/**
 * Liquid Glass bootstrap. On macOS 26+ the plugin gives real glass; on older macOS it
 * falls back to vibrancy; outside Tauri (browser dev) we just paint a solid background.
 * The result is exposed as `data-glass="real" | "vibrancy" | "none"` on <html> so CSS
 * can adjust contrast.
 */
import { isTauri } from "../api/runtime";

export type GlassMode = "real" | "vibrancy" | "none";

export async function initGlass(): Promise<GlassMode> {
  let mode: GlassMode = "none";
  if (isTauri()) {
    try {
      const { isGlassSupported, setLiquidGlassEffect, GlassMaterialVariant } = await import("tauri-plugin-liquid-glass-api");
      await setLiquidGlassEffect({ cornerRadius: 16, variant: GlassMaterialVariant.Regular });
      mode = (await isGlassSupported()) ? "real" : "vibrancy";
    } catch (err) {
      console.warn("Liquid glass unavailable, continuing without it", err);
      mode = "none";
    }
  }
  document.documentElement.dataset.glass = mode;
  return mode;
}
