// Create a new types file to centralize interfaces
export interface Process {
  pid: number;
  ppid: number;
  name: string;
  cpu_usage: number;
  memory_usage: number;
  status: string;
  user: string;
  command: string;
  threads?: number;
  environ: string[];
  root: string;
  virtual_memory: number;
  start_time: number;
  run_time: number;
  disk_usage: [number, number]; // [read_bytes, written_bytes]
  session_id?: number;
}

export interface SystemStats {
  cpu_usage: number[];
  memory_total: number;
  memory_used: number;
  memory_free: number;
  memory_cached: number;
  uptime: number;
  load_avg: [number, number, number];
  network_rx_bytes: number;
  network_tx_bytes: number;
  disk_total_bytes: number;
  disk_used_bytes: number;
  disk_free_bytes: number;
}

export interface Column {
  id: keyof Process;
  label: string;
  visible: boolean;
  required?: boolean;
  format?: (value: any) => string;
}

export interface Theme {
  name: string;
  label: string;
  colors: {
    base: string;
    mantle: string;
    crust: string;
    text: string;
    subtext0: string;
    subtext1: string;
    surface0: string;
    surface1: string;
    surface2: string;
    overlay0: string;
    overlay1: string;
    blue: string;
    lavender: string;
    sapphire: string;
    sky: string;
    red: string;
    maroon: string;
    peach: string;
    yellow: string;
    green: string;
    teal: string;
  };
}

export type Language = "auto" | "en" | "zh-CN";

/** View mode of the network ports modal. */
export type PortsViewMode = "flat" | "grouped" | "tree";

export interface AppConfig {
  language: Language;
  appearance: {
    columnVisibility: Record<string, boolean>;
    /** Manually resized column widths in px, keyed by column id. */
    columnWidths: Record<string, number>;
    highlighting: {
      enabled: boolean;
      durationMs: number;
    };
  };
  behavior: {
    itemsPerPage: number;
    refreshRate: number;
    defaultStatusFilter: string;
    /** Last selected network ports modal view mode. */
    portsViewMode: PortsViewMode;
    /** Favorite ports of the network ports modal, keyed "protocol:local_port". */
    portsFavorites: string[];
    /** Port numbers the user watches; a toast fires when a process starts listening on one. */
    portsWatched: number[];
    /** User-assigned labels per local port number (JSON keys are strings). */
    portLabels: Record<string, string>;
    /** Startup panel hides Windows built-in entries (services/tasks under
     * system paths) by default; the chip toggles them back. */
    startupHideSystem: boolean;
  };
}

export interface ColumnDefinition {
  id: string;
  label: string;
  visible: boolean;
  required?: boolean;
}

export interface StatusOption {
  value: string;
  label: string;
}

export interface RefreshRateOption {
  value: number;
  label: string;
}

export interface ToolBarProps {
  searchTerm: string;
  statusFilter: string;
  itemsPerPage: number;
  currentPage: number;
  totalPages: number;
  totalResults: number;
  columns: ColumnDefinition[];
  refreshRate: number;
  isFrozen: boolean;
}

export interface SortConfig {
  field: keyof Process;
  direction: "asc" | "desc";
}

/** One flattened row of the process tree view. */
export interface ProcessTreeRow {
  process: Process;
  /**
   * Root-to-node name chain identifying the row's subtree; the collapse
   * state key. Name-based (not PID) so collapsed groups survive the PID
   * churn of short-lived child processes.
   */
  path: string;
  /** Nesting depth, starting at 0 for root-level processes. */
  depth: number;
  /** Whether the row has children in the current view. */
  hasChildren: boolean;
  /** Whether the children are currently shown (collapse arrow state). */
  expanded: boolean;
}

export interface PortConnection {
  protocol: string;
  local_addr: string;
  local_port: number;
  remote_addr: string;
  remote_port: number;
  state: string;
  pid: number;
  /** Cumulative bytes sent over this TCP connection (0 for UDP). */
  bytes_sent: number;
  /** Cumulative bytes received over this TCP connection (0 for UDP). */
  bytes_received: number;
}

/** What an HTTP-speaking listener revealed about itself (port probe). */
export interface HttpProbe {
  /** Status code of the `HEAD /` response. */
  status: number;
  /** Server response header, when the listener sends one. */
  server?: string | null;
  /** WWW-Authenticate header present (auth-guarded REST API). */
  www_authenticate: boolean;
  /** `GET /version` answered with version-shaped JSON (clash/mihomo controller). */
  version_json: boolean;
  /** `GET /version` answered 401 (secret-guarded controller API). */
  version_auth: boolean;
}

/** Mechanical probe findings for one TCP listener (port_probe.rs). */
export interface PortProbe {
  /** Answered a TLS ClientHello. */
  tls: boolean;
  /** Completed the SOCKS5 method negotiation. */
  socks5: boolean;
  /** Answered an ordinary HTTP request (HEAD /). */
  http?: HttpProbe | null;
  /** Answered a DNS-over-TCP query. */
  dns: boolean;
}

/** One autostart entry of the startup-items panel (registry Run key,
 * Startup folder file or scheduled task). */
export interface StartupItem {
  /** Stable identity prefixed by kind, used for enable/disable/delete. */
  id: string;
  /** "registry" | "folder" | "task" */
  kind: string;
  name: string;
  /** Registry value data, folder file name, or task exec command. */
  command: string;
  location: string;
  enabled: boolean;
  /** Raw task trigger token ("logon", "boot", ...), else empty. */
  detail: string;
}

/** A process currently holding a file open (Windows Restart Manager). */
export interface FileLocker {
  pid: number;
  /** Friendly application name reported by the Restart Manager. */
  app_name: string;
  /** Service short name, empty for non-service processes. */
  short_name: string;
}

/** One Windows service as reported by the Service Control Manager. */
export interface ServiceInfo {
  /** Internal service name; the key used by control_service. */
  name: string;
  /** Localized display name. */
  display_name: string;
  /** "running" | "stopped" | "paused" | *_pending | "unknown". */
  status: string;
  /** "auto" | "manual" | "disabled" | "boot" | "system" | "unknown". */
  start_type: string;
  /** PID of the hosting process (0 when stopped); shared for svchost. */
  pid: number;
  /** Full binary path from the service configuration. */
  binary_path: string;
}

/** A top-level window of the session (Windows only). */
export interface AppWindow {
  /** Window handle (HWND); the key used by show_window. */
  id: number;
  title: string;
  pid: number;
  /** Executable file name of the owning process, empty when unavailable. */
  process_name: string;
  is_visible: boolean;
  is_minimized: boolean;
}

/** Version-resource metadata and elevation of one process. */
export interface ProcessMetadata {
  /** Full path of the executable image. */
  exe_path: string;
  /** Company name from the version resource; empty when missing. */
  company: string;
  /** File description from the version resource; empty when missing. */
  description: string;
  /** File version from the version resource; empty when missing. */
  version: string;
  /** Whether the process runs with elevated privileges. */
  elevated: boolean;
  /** True when the exe path is known but the file is gone from disk. */
  binary_missing: boolean;
  /** Authenticode verdict: true signed and trusted, false unsigned, null when the check could not run. */
  signed: boolean | null;
}

/** One module (DLL) loaded by a process (Windows only). */
export interface ModuleInfo {
  /** File name of the module, e.g. "kernel32.dll". */
  name: string;
  /** Full path of the module on disk. */
  path: string;
  /** Size of the mapped image in bytes (0 when unknown). */
  size: number;
  /** Base address of the mapped image. */
  base_address: number;
}

/** One kernel-mode driver loaded on the system (Windows only). */
export interface DriverInfo {
  /** Base name of the driver image, e.g. "ntoskrnl.exe". */
  name: string;
  /** Full path of the driver image; empty when unreadable. */
  path: string;
  /** Base address of the loaded image. */
  base_address: number;
}

/** One performance history sample of the selected process. */
export interface PerformanceSample {
  /** CPU usage percentage (0-100). */
  cpu: number;
  /** Physical memory usage in bytes. */
  memory: number;
  /** Bytes read since the previous sample (0 for the first one). */
  disk_read: number;
  /** Bytes written since the previous sample (0 for the first one). */
  disk_write: number;
}
