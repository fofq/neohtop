import { writable } from "svelte/store";
import type { AppConfig } from "$lib/types";
import { DEFAULT_CONFIG } from "$lib/definitions/settings";

function createSettingsStore() {
  const { subscribe, set, update } = writable<AppConfig>(DEFAULT_CONFIG);

  return {
    subscribe,
    init: () => {
      if (typeof window !== "undefined") {
        const stored = localStorage.getItem("neohtop_config");
        if (stored) {
          try {
            const config = JSON.parse(stored);
            // Deep-merge nested groups so configs stored by older versions
            // pick up new defaults (e.g. appearance.highlighting).
            set({
              ...DEFAULT_CONFIG,
              ...config,
              appearance: {
                ...DEFAULT_CONFIG.appearance,
                ...config.appearance,
              },
              behavior: { ...DEFAULT_CONFIG.behavior, ...config.behavior },
            });
          } catch (e) {
            console.error("Failed to parse stored config:", e);
            set(DEFAULT_CONFIG);
          }
        }
      }
    },
    updateConfig: (newConfig: Partial<AppConfig>) => {
      update((config) => {
        const updated = { ...config, ...newConfig };
        if (typeof window !== "undefined") {
          localStorage.setItem("neohtop_config", JSON.stringify(updated));
        }
        return updated;
      });
    },
  };
}

export const settingsStore = createSettingsStore();
