import type { AppConfig } from "$lib/types";

export const DEFAULT_CONFIG: AppConfig = {
  language: "auto",
  appearance: {
    columnVisibility: {
      name: true,
      pid: true,
      status: true,
      user: true,
      cpu_usage: true,
      memory_usage: true,
      virtual_memory: true,
      disk_usage: true,
      ppid: false,
      root: false,
      command: false,
      environ: false,
      session_id: false,
      start_time: false,
      run_time: true,
    },
    // Manually dragged column widths; empty until the user resizes a
    // column (the name column auto-fits from row content every launch).
    columnWidths: {},
    highlighting: {
      enabled: true,
      durationMs: 1000,
    },
  },
  behavior: {
    itemsPerPage: 15,
    refreshRate: 3000,
    defaultStatusFilter: "all",
    treeGrouping: "structure",
    portsViewMode: "flat",
    portsFavorites: [],
    portsWatched: [],
    portLabels: {},
    startupHideSystem: true,
  },
};
