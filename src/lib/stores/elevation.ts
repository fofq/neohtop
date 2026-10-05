import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/** Whether the app itself runs with elevated privileges (admin/root). */
export const isElevated = writable(false);

/**
 * Queries the backend for the elevation state once at startup.
 * A failed query is logged and reported as not elevated, so the
 * settings menu keeps offering the privileged relaunch.
 */
export async function initElevation(): Promise<void> {
  try {
    isElevated.set(await invoke<boolean>("is_elevated"));
  } catch (e) {
    console.error("Failed to query elevation status:", e);
  }
}
