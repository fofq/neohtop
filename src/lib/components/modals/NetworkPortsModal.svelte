<script lang="ts">
  import { onDestroy } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-shell";
  import Fa from "svelte-fa";
  import {
    faBan,
    faBell,
    faCaretRight,
    faCheck,
    faBolt,
    faCircleCheck,
    faCircleInfo,
    faCompressArrowsAlt,
    faCopy,
    faChartLine,
    faCrosshairs,
    faLayerGroup,
    faList,
    faPause,
    faPlay,
    faRefresh,
    faSitemap,
    faSpinner,
    faStethoscope,
    faStar,
    faTriangleExclamation,
    faUpRightFromSquare,
    faXmark,
  } from "@fortawesome/free-solid-svg-icons";
  import { Modal } from "$lib/components";
  import { t } from "$lib/i18n";
  import {
    processStore,
    settingsStore,
    ensureProcessMetadata,
    isElevated,
  } from "$lib/stores/index";
  import {
    formatBytes,
    formatDate,
    formatPercentage,
    formatUptime,
    withElevationHint,
  } from "$lib/utils";
  import type {
    PortConnection,
    PortProbe,
    PortsViewMode,
    Process,
    ProcessMetadata,
  } from "$lib/types";

  export let show = false;
  export let onClose: () => void;

  let connections: PortConnection[] = [];
  let isLoading = false;
  let error: string | null = null;
  let searchTerm = "";

  type ProtocolFilter = "all" | "tcp" | "udp";
  type StateBucket = "listen" | "established" | "other";
  type StateFilter = "all" | StateBucket;
  type SortField =
    | "local_port"
    | "remote_addr"
    | "state"
    | "pid"
    | "process"
    | "download"
    | "upload";
  type SortDirection = "asc" | "desc";
  type ViewMode = PortsViewMode;

  interface ProcessPortGroup {
    pid: number;
    name: string;
    count: number;
    ports: number[];
    connections: PortConnection[];
  }

  /** One process node of the ports ancestry tree. */
  interface PortTreeNode {
    pid: number;
    /** Parent PID from the snapshot; null/0 when the parent is unknown. */
    ppid: number | null;
    name: string;
    /** Connections owned by this process (post-filter), port-sorted. */
    connections: PortConnection[];
    ports: number[];
    ownCount: number;
    /** Own connections plus every descendant's, computed bottom-up. */
    totalCount: number;
    parent: PortTreeNode | null;
    children: PortTreeNode[];
  }

  /** One flattened renderable row of the tree view. */
  type PortTreeRow =
    | { kind: "node"; depth: number; node: PortTreeNode }
    | { kind: "connection"; depth: number; connection: PortConnection };

  // Quick filters, view mode and sorting; kept in component state so they
  // survive closing and reopening the modal. The view mode is also mirrored
  // into AppConfig so it survives app restarts.
  let protocolFilter: ProtocolFilter = "all";
  let stateFilter: StateFilter = "all";
  // Derived from the persisted config (setViewMode writes it back on every
  // change) instead of captured once at init, so the modal follows the store
  // and a stale value from localStorage falls back to the flat list instead
  // of rendering the tree branch with column sorting still looking enabled
  $: viewMode = normalizeViewMode($settingsStore.behavior.portsViewMode);
  let sortField: SortField | null = null;
  let sortDirection: SortDirection = "asc";
  let focusedPid: number | null = null;
  let focusedName = "";
  let expandedGroups: Set<number> = new Set();
  // Expanded PIDs of the tree view; component state, so expansions survive
  // auto-refreshes and reopenings. Nodes not in the set render collapsed
  // (the default folded-to-first-level state).
  let expandedTreeNodes: Set<number> = new Set();

  // Favorite ports ("protocol:local_port" keys), persisted in AppConfig and
  // pinned to the top of the list and grouped views
  let favoritesOnly = false;
  // Hides connections without any reported traffic (listeners etc.)
  let hideIdle = false;

  // --- Port categories: two-layer detection -------------------------------
  // Layer 1 (the accuracy lever, port-killer's approach): the owning
  // process's name — postgres on an unusual port is still a database,
  // mihomo on a custom port is still a proxy. Layer 2 (fallback for
  // unattributed or generically-named processes): well-known local ports.
  type PortCategoryKey =
    | "web"
    | "database"
    | "dev"
    | "system"
    | "proxy"
    | "mail";

  /** Process-name patterns per category, checked in order against the
   * lowercased executable name (".exe" stripped) via startsWith — every
   * pattern here is distinctive enough that a prefix match is meaningful
   * (generic words like "go" or "system" are deliberately absent). */
  const PROCESS_CATEGORIES: Array<{
    key: PortCategoryKey;
    patterns: string[];
  }> = [
    // Exact-name hits only: these words are far too generic as prefixes
    {
      key: "system",
      patterns: [
        "svchost",
        "lsass",
        "csrss",
        "services",
        "wininit",
        "smss",
        "system",
        "spoolsv",
        "dwm",
        "winlogon",
        "fontdrvhost",
        "sihost",
        "taskhostw",
      ],
    },
    {
      key: "proxy",
      patterns: [
        "mihomo",
        "clash",
        "v2ray",
        "xray",
        "sing-box",
        "singbox",
        "hysteria",
        "shadowsocks",
        "trojan-go",
        "naive",
        "privoxy",
        "tinyproxy",
        "squid",
        "3proxy",
        "dante",
        "proxifier",
        "leaf",
        "juicity",
        "tuic",
      ],
    },
    {
      key: "database",
      patterns: [
        "postgres",
        "mysql",
        "mariadb",
        "redis",
        "mongo",
        "memcached",
        "cockroach",
        "clickhouse",
        "cassandra",
        "elastic",
        "sqlservr",
        "oracle",
        "influxd",
        "rabbitmq",
      ],
    },
    {
      key: "web",
      patterns: [
        "nginx",
        "apache",
        "httpd",
        "caddy",
        "traefik",
        "lighttpd",
        "haproxy",
        "tomcat",
        "w3wp",
        "frps",
        "frpc",
      ],
    },
    {
      key: "dev",
      patterns: [
        "node",
        "deno",
        "bun",
        "npm",
        "pnpm",
        "yarn",
        "python",
        "ruby",
        "php",
        "java",
        "kotlin",
        "scala",
        "cargo",
        "rustc",
        "dotnet",
        "vite",
        "webpack",
        "esbuild",
        "parcel",
        "dart",
        "flutter",
        "code",
      ],
    },
  ];

  const PORT_CATEGORIES: Array<{ key: PortCategoryKey; ports: number[] }> = [
    {
      key: "web",
      ports: [80, 443, 593, 8080, 8081, 8443, 8888, 8880],
    },
    {
      key: "database",
      ports: [
        1433, 1521, 3306, 5432, 5984, 6379, 7474, 8086, 9042, 9092, 9200, 11211,
        27017,
      ],
    },
    {
      key: "dev",
      ports: [
        3000, 3001, 3333, 4000, 4001, 4200, 5000, 5001, 5173, 5174, 7000, 8000,
        8001, 9000, 9001, 9229, 9230,
      ],
    },
    {
      key: "system",
      ports: [
        22, 53, 69, 88, 123, 135, 139, 161, 389, 445, 464, 514, 636, 1900, 3268,
        3269, 3389, 5353, 5355, 5900, 5985, 5986,
      ],
    },
    {
      key: "proxy",
      ports: [1080, 3128, 8118, 8388, 9090, 10808, 10809, 7890, 7891, 7897],
    },
    {
      key: "mail",
      ports: [25, 110, 143, 465, 587, 993, 995],
    },
  ];

  /** Lowercases the owning process's executable name with the extension
   * stripped ("MIHOMO-ALPHA.EXE" → "mihomo-alpha"), or null when unknown. */
  function normalizedProcessName(pid: number): string | null {
    const raw = processNameByPid.get(pid);
    if (!raw) return null;
    return raw.toLowerCase().replace(/\.(exe|com|bat|cmd)$/, "");
  }

  /** Category implied by the owning process's name alone (the port table
   * is NOT consulted) — also drives the probe-role interpretation. */
  function processCategoryOf(pid: number): PortCategoryKey | null {
    const name = normalizedProcessName(pid);
    if (!name) return null;
    for (const category of PROCESS_CATEGORIES) {
      for (const pattern of category.patterns) {
        if (name === pattern || name.startsWith(pattern)) {
          return category.key;
        }
      }
    }
    return null;
  }

  /** First matching category for a connection: the owning process's name
   * wins (exact-or-prefix hit on a distinctive pattern), then the
   * well-known local-port table. Null for ordinary ports. */
  function categoryOf(connection: PortConnection): PortCategoryKey | null {
    return (
      processCategoryOf(connection.pid) ??
      PORT_CATEGORIES.find((category) =>
        category.ports.includes(connection.local_port),
      )?.key ??
      null
    );
  }
  let categoryFilter: "all" | PortCategoryKey = "all";

  // --- Listening-port role probing ----------------------------------------
  // One on-demand probe battery per listener ("what does this port
  // actually speak?"), cached for the modal session keyed by pid:port so
  // the IPv4/IPv6 wildcard pair shares one result. Results are never
  // probed automatically for the whole table — only when the user clicks
  // a row's identify button or expands the detail panel.
  type PortRole =
    | "socks"
    | "http_proxy"
    | "mixed"
    | "controller"
    | "web"
    | "tls"
    | "dns";
  type PortRoleVerdict = {
    tag: PortRole | null;
    /** What the open-in-browser button may do: https for TLS listeners,
     * panel for clash-style controllers (opens /ui), http for web. */
    open: "http" | "https" | "panel" | null;
  };
  let roleCache = new Map<string, PortProbe>();
  let roleLoading = new Set<string>();
  /** Bumped whenever the cache or loading set changes; referenced by the
   * derived role map so Svelte re-renders the tags/buttons. */
  let roleCacheVersion = 0;

  function roleKeyOf(connection: PortConnection): string {
    return `${connection.pid}:${connection.local_port}`;
  }

  /** Probe target for a listener: wildcard binds are reachable on the
   * loopback, specific addresses are probed as shown. */
  function probeTargetOf(connection: PortConnection): string {
    const addr = connection.local_addr;
    if (addr === "0.0.0.0" || addr === "::" || addr === "") return "127.0.0.1";
    return addr;
  }

  async function identifyPortRole(connection: PortConnection) {
    const key = roleKeyOf(connection);
    if (roleCache.has(key) || roleLoading.has(key)) return;
    roleLoading.add(key);
    roleCacheVersion++;
    try {
      const probe: PortProbe = await invoke("identify_port", {
        host: probeTargetOf(connection),
        port: connection.local_port,
      });
      roleCache.set(key, probe);
    } catch {
      // Identification is best effort; silence keeps rows untagged
    } finally {
      roleLoading.delete(key);
      roleCacheVersion++;
    }
  }

  /** Interprets the mechanical probe findings into a role tag and an
   * open-in-browser verdict, using the owning process as context: a
   * version-JSON endpoint on a proxy core is a controller, on anything
   * else it is just a web endpoint. */
  function derivePortRole(probe: PortProbe, pid: number): PortRoleVerdict {
    if (probe.tls) return { tag: "tls", open: "https" };
    if (probe.dns) return { tag: "dns", open: null };
    const isProxyProcess = processCategoryOf(pid) === "proxy";
    if (probe.socks5 && probe.http) return { tag: "mixed", open: null };
    if (probe.socks5) return { tag: "socks", open: null };
    if (probe.http) {
      const controllerish =
        probe.http.version_json ||
        (probe.http.www_authenticate && probe.http.status === 401);
      if (controllerish && isProxyProcess) {
        return { tag: "controller", open: "panel" };
      }
      if (isProxyProcess) return { tag: "http_proxy", open: null };
      return { tag: "web", open: "http" };
    }
    return { tag: null, open: null };
  }

  /** Derived tag/verdict per probed listener; depends on roleCacheVersion
   * so the template re-renders when async probes land. */
  $: derivedRoles = (() => {
    void roleCacheVersion;
    const out = new Map<string, PortRoleVerdict>();
    for (const [key, probe] of roleCache) {
      const pid = Number(key.slice(0, key.indexOf(":")));
      out.set(key, derivePortRole(probe, pid));
    }
    return out;
  })();

  /** Keys currently being probed (for the spinner state). */
  $: probingKeys = (() => {
    void roleCacheVersion;
    return new Set(roleLoading);
  })();

  /** Keys with a finished probe (for the identify button's active state). */
  $: identifiedKeys = (() => {
    void roleCacheVersion;
    return new Set(roleCache.keys());
  })();

  function portRoleOf(connection: PortConnection): PortRoleVerdict | null {
    return derivedRoles.get(roleKeyOf(connection)) ?? null;
  }

  /** Security-relevant bind scope straight from the address: wildcard
   * binds accept connections from any interface (LAN included), loopback
   * binds are local-only. Specific adapter addresses get no badge. */
  function bindScopeOf(
    connection: PortConnection,
  ): "ports.bindAll" | "ports.bindLocal" | null {
    const addr = connection.local_addr;
    if (addr === "0.0.0.0" || addr === "::") return "ports.bindAll";
    if (addr === "127.0.0.1" || addr === "::1") return "ports.bindLocal";
    return null;
  }

  // --- Port deep-dive panel ("Why is this running?") ---
  // The panel is keyed by the connection's 6-tuple instead of object
  // identity, so it stays open across auto-refreshes (which replace the
  // connection objects wholesale) and collapses on its own when the
  // connection disappears from a refresh.
  type DetailWarning = { key: string; tone: "red" | "yellow" };

  /** One breadcrumb segment of the ancestry chain; the last one is the subject. */
  interface ChainSegment {
    pid: number;
    name: string;
    /** Command line shown on hover; null when the snapshot lacks it. */
    command: string | null;
    isSubject: boolean;
  }

  /** How far the ppid walk may go before the chain is cut off. */
  const MAX_CHAIN_DEPTH = 32;

  let detailKey: string | null = null;
  let detailMetadata: ProcessMetadata | null = null;
  /** PID the metadata was loaded for, so refreshes of the same process
   * don't re-trigger the (cached) backend call. */
  let metadataPid: number | null = null;
  let detailError: string | null = null;
  let isTogglingSuspend = false;

  // Kill confirmation, mirroring the close-connection confirmation flow;
  // the message names the exact process and PID to avoid killing a
  // look-alike instance
  let processToKill: { pid: number; name: string } | null = null;
  let isKillingProcess = false;
  let killError: string | null = null;

  function connectionKey(connection: PortConnection): string {
    return [
      connection.protocol,
      connection.local_addr,
      connection.local_port,
      connection.remote_addr,
      connection.remote_port,
      connection.pid,
    ].join("|");
  }

  function portKeyOf(connection: PortConnection): string {
    return `${connection.protocol.toLowerCase()}:${connection.local_port}`;
  }

  function togglePortFavorite(key: string) {
    const current = $settingsStore.behavior.portsFavorites ?? [];
    const next = current.includes(key)
      ? current.filter((k) => k !== key)
      : [...current, key];
    settingsStore.updateConfig({
      behavior: { ...$settingsStore.behavior, portsFavorites: next },
    });
  }

  /** A group heads the ports of several connections: starring it stars all
   * of its ports (remove-all when any of them is already favorited). */
  function toggleGroupFavorites(connections: PortConnection[]) {
    const keys = Array.from(new Set(connections.map(portKeyOf)));
    const current = $settingsStore.behavior.portsFavorites ?? [];
    const remove = keys.some((key) => current.includes(key));
    const next = remove
      ? current.filter((key) => !keys.includes(key))
      : [...current, ...keys.filter((key) => !current.includes(key))];
    settingsStore.updateConfig({
      behavior: { ...$settingsStore.behavior, portsFavorites: next },
    });
  }

  /** Watches a local port number: the app polls listening ports and
   * raises a toast when a process starts listening on it. */
  function togglePortWatch(port: number) {
    const current = $settingsStore.behavior.portsWatched ?? [];
    const next = current.includes(port)
      ? current.filter((p) => p !== port)
      : [...current, port];
    settingsStore.updateConfig({
      behavior: { ...$settingsStore.behavior, portsWatched: next },
    });
  }

  // Favorites first (stable partition); without favorites the list is
  // returned untouched so sorting keeps its exact order
  function partitionByFavorites(
    list: PortConnection[],
    favorites: Set<string>,
  ): PortConnection[] {
    if (favorites.size === 0) return list;
    const head: PortConnection[] = [];
    const tail: PortConnection[] = [];
    for (const connection of list) {
      (favorites.has(portKeyOf(connection)) ? head : tail).push(connection);
    }
    return head.length === 0 ? list : head.concat(tail);
  }

  function partitionGroupsByFavorites(
    groups: ProcessPortGroup[],
    favorites: Set<string>,
  ): ProcessPortGroup[] {
    if (favorites.size === 0) return groups;
    const head: ProcessPortGroup[] = [];
    const tail: ProcessPortGroup[] = [];
    for (const group of groups) {
      const isFavorite = group.connections.some((connection) =>
        favorites.has(portKeyOf(connection)),
      );
      (isFavorite ? head : tail).push(group);
    }
    return head.length === 0 ? groups : head.concat(tail);
  }

  /**
   * Walks the ppid chain from the panel's process up through the snapshot,
   * oldest ancestor first, with the subject process as the final segment.
   * Cycles and self-references stop the walk; a parent that is not in the
   * snapshot marks the chain broken ("parent has exited or is not visible").
   */
  function buildAncestryChain(
    process: Process,
    processes: Process[],
  ): { segments: ChainSegment[]; broken: boolean } {
    const byPid = new Map(processes.map((entry) => [entry.pid, entry]));
    const segments: ChainSegment[] = [
      {
        pid: process.pid,
        name: process.name,
        command: process.command || null,
        isSubject: true,
      },
    ];
    const visited = new Set<number>([process.pid]);
    let broken = false;
    let current = process;
    for (let depth = 0; depth < MAX_CHAIN_DEPTH; depth++) {
      const ppid = current.ppid;
      if (!ppid || ppid === current.pid || visited.has(ppid)) break;
      const parent = byPid.get(ppid);
      if (!parent) {
        broken = true;
        break;
      }
      visited.add(ppid);
      segments.unshift({
        pid: parent.pid,
        name: parent.name,
        command: parent.command || null,
        isSubject: false,
      });
      current = parent;
    }
    return { segments, broken };
  }

  // Warnings follow witr: security-relevant facts first (red), resource
  // facts second (yellow). The wildcard-listen check spans every connection
  // of the process, not just the one the panel was opened from.
  function buildDetailWarnings(
    process: Process | null,
    metadata: ProcessMetadata | null,
    pidConnections: PortConnection[],
  ): DetailWarning[] {
    const warnings: DetailWarning[] = [];
    const exposesToNetwork = pidConnections.some(
      (entry) =>
        stateBucketOf(entry.state) === "listen" &&
        (entry.local_addr === "0.0.0.0" || entry.local_addr === "::"),
    );
    if (exposesToNetwork) {
      warnings.push({ key: "ports.warnWildcard", tone: "red" });
    }
    if (metadata?.elevated) {
      warnings.push({ key: "ports.warnElevated", tone: "red" });
    }
    if (process && process.memory_usage > 1073741824) {
      warnings.push({ key: "ports.warnMemory", tone: "yellow" });
    }
    if (process && process.run_time > 7776000) {
      warnings.push({ key: "ports.warnRuntime", tone: "yellow" });
    }
    return warnings;
  }

  function toggleDetail(connection: PortConnection) {
    const key = connectionKey(connection);
    detailKey = detailKey === key ? null : key;
  }

  function toggleDetailSuspend() {
    const process = detailProcess;
    if (!process || isTogglingSuspend) return;
    isTogglingSuspend = true;
    detailError = null;
    (async () => {
      try {
        const wasSuspended = $processStore.suspendedPids.has(process.pid);
        const command = wasSuspended ? "resume_process" : "suspend_process";
        const success = await invoke<boolean>(command, { pid: process.pid });
        if (!success) {
          throw new Error(
            wasSuspended
              ? "Failed to resume process"
              : "Failed to suspend process",
          );
        }
        // Mirror the suspend marker into the shared store so the main
        // process table keeps showing the suspended state
        processStore.update((state) => {
          const suspendedPids = new Set(state.suspendedPids);
          if (wasSuspended) {
            suspendedPids.delete(process.pid);
          } else {
            suspendedPids.add(process.pid);
          }
          return { ...state, suspendedPids };
        });
      } catch (e) {
        detailError = withElevationHint(
          e instanceof Error ? e.message : String(e),
        );
      } finally {
        isTogglingSuspend = false;
      }
    })();
  }

  // A kill can only be offered for a process the current snapshot knows:
  // synthetic "-" nodes (PID absent from the snapshot) must not expose the
  // entry, and a PID that already left the snapshot must not be confirmed
  // against a stale name (the PID may have been reused by now)
  function canKillProcess(pid: number): boolean {
    return pid > 0 && processNameByPid.has(pid);
  }

  function confirmKillProcess(pid: number, name: string) {
    if (!canKillProcess(pid)) return;
    processToKill = { pid, name };
    killError = null;
  }

  function cancelKillProcess() {
    processToKill = null;
    killError = null;
  }

  async function handleKillProcess() {
    const target = processToKill;
    if (!target || isKillingProcess) return;
    isKillingProcess = true;
    killError = null;
    try {
      const success = await invoke<boolean>("kill_process", {
        pid: target.pid,
      });
      if (!success) {
        throw new Error("Failed to kill process");
      }
      processToKill = null;
      // Refresh so the dead process's connections disappear from the list
      await loadConnections();
    } catch (e) {
      // Permission failures get the administrator-relaunch hint appended
      killError = withElevationHint(e instanceof Error ? e.message : String(e));
    } finally {
      isKillingProcess = false;
    }
  }

  /** Guards against a corrupted or outdated persisted view mode: unknown
   * values fall back to the flat list. */
  function normalizeViewMode(value: string): ViewMode {
    return value === "grouped" || value === "tree" ? value : "flat";
  }

  function setViewMode(mode: ViewMode) {
    if (mode === "tree") {
      // The tree orders itself by aggregate connection count; drop any
      // column sort so the header state matches what is displayed
      sortField = null;
      sortDirection = "asc";
    }
    settingsStore.updateConfig({
      behavior: { ...$settingsStore.behavior, portsViewMode: mode },
    });
  }

  // Close-connection confirmation, mirroring the kill confirmation flow
  let connectionToClose: PortConnection | null = null;
  // Name frozen when the confirmation opened, like processToKill.name, so a
  // snapshot refresh (or a reused PID) can't re-render the dialog with a
  // process name that no longer matches the shown connection endpoints
  let connectionToCloseName = "";
  let isClosingConnection = false;
  let closeError: string | null = null;

  // Resolve PIDs against the current process snapshot
  $: processNameByPid = new Map(
    $processStore.processes.map((process) => [process.pid, process.name]),
  );
  // Parent PIDs of the snapshot, used to grow ancestor chains in tree view
  $: processPpidByPid = new Map(
    $processStore.processes.map((process) => [process.pid, process.ppid]),
  );
  // Favorite "protocol:local_port" keys from the persisted config
  $: favoriteKeys = new Set($settingsStore.behavior.portsFavorites ?? []);

  // Watched local ports (bell toggle) and the subset of them that the
  // current snapshot shows a LISTEN row for, so watched chips can read
  // live vs idle at a glance
  $: watchedPorts = $settingsStore.behavior.portsWatched ?? [];
  $: liveListenPorts = new Set(
    connections
      .filter((connection) => connection.state === "LISTEN")
      .map((connection) => connection.local_port),
  );

  // --- Per-connection traffic speeds ---
  // The backend reports cumulative per-TCP-connection byte counters from
  // two sources: the slow full snapshot (main refresh rate) and the fast
  // "real-time" heartbeat that only polls the currently visible rows.
  // Each counter update is timestamped per key, so the two cadences never
  // fight over a global interval, and the instant speed is EMA-smoothed
  // so 1s TCP bursts render as a stable number instead of flicker.
  interface CounterSample {
    sent: number;
    received: number;
    at: number;
  }
  let lastTrafficByKey = new Map<string, CounterSample>();
  let speedByKey: Record<string, { down: number; up: number }> = {};

  /** Instant speed from two samples, folded into the previous smoothed
   * value; long gaps (frozen app, closed modal) skip smoothing. */
  function absorbCounters(
    key: string,
    sent: number,
    received: number,
    now: number,
  ): boolean {
    const prev = lastTrafficByKey.get(key);
    lastTrafficByKey.set(key, { sent, received, at: now });
    if (!prev) return false;
    const dt = (now - prev.at) / 1000;
    if (dt <= 0) return false;
    const instant = {
      down: Math.max(0, (received - prev.received) / dt),
      up: Math.max(0, (sent - prev.sent) / dt),
    };
    const old = speedByKey[key] ?? { down: 0, up: 0 };
    const smoothed =
      dt < 10 && (old.down > 0 || old.up > 0)
        ? {
            down: old.down * 0.5 + instant.down * 0.5,
            up: old.up * 0.5 + instant.up * 0.5,
          }
        : instant;
    const changed =
      Math.abs(smoothed.down - old.down) > 0.5 ||
      Math.abs(smoothed.up - old.up) > 0.5 ||
      received !== prev.received ||
      sent !== prev.sent;
    if (changed) {
      speedByKey = { ...speedByKey, [key]: smoothed };
    }
    return changed;
  }

  /** Slow snapshot: absorb every connection's counters (rows the
   * heartbeat does not cover still get main-cadence speeds) */
  function computeTrafficSpeeds(list: PortConnection[]) {
    const now = Date.now();
    let changed = false;
    const seen = new Set<string>();
    for (const connection of list) {
      const key = connectionKey(connection);
      seen.add(key);
      changed =
        absorbCounters(
          key,
          connection.bytes_sent,
          connection.bytes_received,
          now,
        ) || changed;
    }
    // Drop entries for connections that left the list so the map cannot
    // grow across hours of auto-refresh
    if (lastTrafficByKey.size !== seen.size) {
      for (const key of [...lastTrafficByKey.keys()]) {
        if (!seen.has(key)) lastTrafficByKey.delete(key);
      }
    }
    if (changed) {
      speedByKey = { ...speedByKey };
    }
  }

  /** "1.2 KB/s" for a speed, "-" when there is nothing flowing */
  function speedText(bytesPerSec: number): string {
    if (!bytesPerSec || bytesPerSec < 1) return "-";
    return `${formatBytes(bytesPerSec)}/s`;
  }

  function connectionSpeed(connection: PortConnection): {
    down: number;
    up: number;
  } {
    return speedByKey[connectionKey(connection)] ?? { down: 0, up: 0 };
  }

  /** Sums cumulative bytes and current speeds over a connection group */
  function sumTraffic(list: PortConnection[]): {
    down: number;
    up: number;
    received: number;
    sent: number;
  } {
    let down = 0;
    let up = 0;
    let received = 0;
    let sent = 0;
    for (const connection of list) {
      const speed = connectionSpeed(connection);
      down += speed.down;
      up += speed.up;
      received += connection.bytes_received;
      sent += connection.bytes_sent;
    }
    return { down, up, received, sent };
  }

  async function loadConnections(manual = false) {
    if (isLoading) return;
    isLoading = true;
    manualLoad = manual;
    error = null;
    try {
      connections = await invoke<PortConnection[]>("get_network_ports");
      computeTrafficSpeeds(connections);
    } catch (e) {
      error = withElevationHint(e instanceof Error ? e.message : String(e));
    } finally {
      isLoading = false;
      manualLoad = false;
    }
  }

  let refreshTimer: ReturnType<typeof setInterval> | null = null;
  $: refreshRate = $settingsStore.behavior.refreshRate;

  // --- Real-time traffic heartbeat ---
  // Fast (1s) counter polling for exactly the rows currently on screen;
  // the full snapshot keeps its main-page cadence. Costs one tiny IPC per
  // tick and a handful of ESTATS reads on the backend.
  let realtime = true;
  let manualLoad = false;
  let tickerTimer: ReturnType<typeof setInterval> | null = null;

  // Speeds need ESTATS collection, which requires elevation: without admin
  // every counter reads zero, so the speed columns, the traffic/idle filter,
  // the live heartbeat and the totals bar are all hidden instead
  $: showSpeedCols = $isElevated;

  const TICKER_INTERVAL_MS = 1000;
  /** Upper bound of polled rows per tick — beyond this the snapshot
   * cadence takes over for the tail (keeps IPC bounded). */
  const TICKER_MAX_KEYS = 600;

  function visibleConnectionKeys(): string[] {
    const keys: string[] = [];
    if (viewMode === "flat") {
      for (const connection of visibleConnections) {
        keys.push(connectionKey(connection));
      }
    } else if (viewMode === "grouped") {
      for (const group of displayedGroups) {
        if (expandedGroups.has(group.pid)) {
          for (const connection of group.connections) {
            keys.push(connectionKey(connection));
          }
        }
      }
    } else {
      for (const row of flatTreeRows) {
        if (row.kind === "connection") {
          keys.push(connectionKey(row.connection));
        }
      }
    }
    return [...new Set(keys)].slice(0, TICKER_MAX_KEYS);
  }

  async function tickTraffic() {
    const keys = visibleConnectionKeys();
    if (keys.length === 0) return;
    try {
      const result = await invoke<
        Record<string, { sent: number; received: number }>
      >("get_traffic_counters", { keys });
      const now = Date.now();
      let changed = false;
      for (const [key, sample] of Object.entries(result)) {
        changed =
          absorbCounters(key, sample.sent, sample.received, now) || changed;
      }
      if (changed) {
        speedByKey = { ...speedByKey };
      }
    } catch {
      // Heartbeat failures are silent; the next snapshot heals the state
    }
  }

  function syncTrafficTicker(open: boolean, on: boolean, frozen: boolean) {
    if (tickerTimer !== null) {
      clearInterval(tickerTimer);
      tickerTimer = null;
    }
    if (open && on && !frozen) {
      tickerTimer = setInterval(() => {
        tickTraffic();
      }, TICKER_INTERVAL_MS);
    }
  }

  $: syncTrafficTicker(show, realtime && $isElevated, $processStore.isFrozen);

  // Reload every time the modal is opened
  $: if (show) {
    loadConnections();
  }

  // Auto-refresh while the modal is open, mirroring the main table: same
  // rate and also paused while the main view is frozen. The manual refresh
  // button was dropped — auto refresh made it redundant.
  $: syncAutoRefresh(show, refreshRate, $processStore.isFrozen);

  function syncAutoRefresh(open: boolean, rate: number, frozen: boolean) {
    stopAutoRefresh();
    if (open && !frozen && rate > 0) {
      refreshTimer = setInterval(() => {
        loadConnections();
      }, rate);
    }
  }

  function stopAutoRefresh() {
    if (refreshTimer !== null) {
      clearInterval(refreshTimer);
      refreshTimer = null;
    }
  }

  onDestroy(() => {
    stopAutoRefresh();
    if (tickerTimer !== null) clearInterval(tickerTimer);
    if (portActionNoticeTimer !== null) clearTimeout(portActionNoticeTimer);
    for (const timer of copiedPortTimers.values()) clearTimeout(timer);
    copiedPortTimers.clear();
  });

  // Jumps to the owning process in the main table: this modal closes and
  // the shared details modal opens for the same snapshot entry the process
  // table uses. Reachable from the panel's action row and the group name.
  function openProcessDetails(pid: number) {
    if (!pid) return;
    const process = $processStore.processes.find((p) => p.pid === pid);
    if (!process) return;
    onClose();
    processStore.showProcessDetails(process);
  }

  // Row-level double-click handler: opens (or closes) the in-modal deep-dive
  // panel instead of jumping away. Clicks landing on the row's action
  // buttons must not toggle it
  function handleRowDblClick(event: MouseEvent, connection: PortConnection) {
    if ((event.target as HTMLElement | null)?.closest?.(".focus-btn")) return;
    toggleDetail(connection);
  }

  // A connection can only be closed when the backend can act on it: an
  // established IPv4 TCP connection (SetTcpEntry has no IPv6 equivalent,
  // and only live connections can be reset)
  function canCloseConnection(connection: PortConnection): boolean {
    return (
      connection.protocol.toUpperCase() === "TCP" &&
      connection.state.toUpperCase().includes("ESTABLISH") &&
      connection.remote_addr !== "" &&
      !connection.local_addr.includes(":") &&
      !connection.remote_addr.includes(":")
    );
  }

  function confirmCloseConnection(connection: PortConnection) {
    connectionToClose = connection;
    connectionToCloseName = processNameByPid.get(connection.pid) ?? "-";
    closeError = null;
  }

  function cancelCloseConnection() {
    connectionToClose = null;
    connectionToCloseName = "";
    closeError = null;
  }

  async function handleCloseConnection() {
    const target = connectionToClose;
    if (!target || isClosingConnection) return;
    isClosingConnection = true;
    closeError = null;
    try {
      const success = await invoke<boolean>("close_tcp_connection", {
        localAddr: target.local_addr,
        localPort: target.local_port,
        remoteAddr: target.remote_addr,
        remotePort: target.remote_port,
        pid: target.pid,
      });
      if (!success) {
        throw new Error("Failed to close the connection");
      }
      connectionToClose = null;
      // Refresh so the reset connection disappears from the list
      await loadConnections();
    } catch (e) {
      // Permission failures get the administrator-relaunch hint appended
      closeError = withElevationHint(
        e instanceof Error ? e.message : String(e),
      );
    } finally {
      isClosingConnection = false;
    }
  }

  // --- Port browser actions (PortKiller-style) ---
  // TCP rows in the LISTEN state offer to open http://localhost:{port} in
  // the system browser plus a copy-address button; failures surface as a
  // transient inline hint on the row instead of an alert dialog. Feedback
  // is keyed by the connection 6-tuple so identical rows don't share state
  let copiedPortKeys = new Set<string>();
  const copiedPortTimers = new Map<string, ReturnType<typeof setTimeout>>();
  let portActionNotice: { key: string; message: string } | null = null;
  let portActionNoticeTimer: ReturnType<typeof setTimeout> | null = null;

  /** Only listening TCP ports map to a browsable URL. */
  function isListenablePort(connection: PortConnection): boolean {
    return (
      connection.protocol.toUpperCase() === "TCP" &&
      stateBucketOf(connection.state) === "listen"
    );
  }

  /** URL for a listening TCP port: the exact bind address when it is
   * specific (servers bound to one IP ignore localhost, so opening the
   * shown address is the only reliable choice), friendly localhost for
   * wildcard binds — there the name reaches every interface and the
   * browser races IPv4/IPv6 loopback by itself. IPv6 needs brackets. */
  function portUrl(connection: PortConnection): string {
    const addr = connection.local_addr;
    if (addr === "0.0.0.0" || addr === "::" || addr === "") {
      return `http://localhost:${connection.local_port}`;
    }
    if (addr.includes(":")) {
      return `http://[${addr}]:${connection.local_port}`;
    }
    return `http://${addr}:${connection.local_port}`;
  }

  /** URL the browser should open for an identified listener, or null when
   * the role is not browser-relevant (SOCKS/DNS/unknown). Wildcard binds
   * open as localhost; TLS listeners open as https; clash-style
   * controllers jump straight to the /ui panel. */
  function browserUrlOf(connection: PortConnection): string | null {
    const role = portRoleOf(connection);
    if (!role?.open) return null;
    const addr = connection.local_addr;
    const scheme = role.open === "https" ? "https" : "http";
    let base: string;
    if (addr === "0.0.0.0" || addr === "::" || addr === "") {
      base = `${scheme}://localhost:${connection.local_port}`;
    } else if (addr.includes(":")) {
      base = `${scheme}://[${addr}]:${connection.local_port}`;
    } else {
      base = `${scheme}://${addr}:${connection.local_port}`;
    }
    return role.open === "panel" ? `${base}/ui` : base;
  }

  async function openPortInBrowser(connection: PortConnection) {
    const url = browserUrlOf(connection);
    if (!url) return;
    try {
      await open(url);
    } catch {
      showPortNotice(connectionKey(connection), $t("ports.browserOpenFailed"));
    }
  }

  async function copyPortAddress(connection: PortConnection) {
    const key = connectionKey(connection);
    try {
      await navigator.clipboard.writeText(portUrl(connection));
    } catch {
      showPortNotice(key, $t("ports.copyFailed"));
      return;
    }
    const next = new Set(copiedPortKeys);
    next.add(key);
    copiedPortKeys = next;
    const previous = copiedPortTimers.get(key);
    if (previous) clearTimeout(previous);
    copiedPortTimers.set(
      key,
      setTimeout(() => {
        copiedPortTimers.delete(key);
        const rest = new Set(copiedPortKeys);
        rest.delete(key);
        copiedPortKeys = rest;
      }, 1600),
    );
  }

  function showPortNotice(key: string, message: string) {
    if (portActionNoticeTimer !== null) clearTimeout(portActionNoticeTimer);
    portActionNotice = { key, message };
    portActionNoticeTimer = setTimeout(() => {
      portActionNotice = null;
      portActionNoticeTimer = null;
    }, 2400);
  }

  function filterConnections(
    all: PortConnection[],
    term: string,
    names: Map<number, string>,
  ): PortConnection[] {
    const query = term.trim().toLowerCase();
    if (!query) return all;
    return all.filter((connection) => {
      const processName = (names.get(connection.pid) ?? "").toLowerCase();
      return (
        connection.protocol.toLowerCase().includes(query) ||
        `${connection.local_addr}:${connection.local_port}`
          .toLowerCase()
          .includes(query) ||
        connection.remote_addr.toLowerCase().includes(query) ||
        (connection.remote_port !== 0 &&
          connection.remote_port.toString().includes(query)) ||
        connection.state.toLowerCase().includes(query) ||
        connection.pid.toString().includes(query) ||
        processName.includes(query)
      );
    });
  }

  function stateBucketOf(state: string): StateBucket {
    const value = state.trim().toUpperCase();
    if (value.includes("LISTEN")) return "listen";
    if (value.includes("ESTABLISH")) return "established";
    return "other";
  }

  /** Localized label of a connection state: looks up `netState.<lowercased
   * state>` and falls back to the raw backend string when no dictionary
   * entry exists (UDP's "-", OS-specific values). The translator is passed
   * in from the template so the cells re-render when the locale changes. */
  function localizedState(
    state: string,
    translate: (key: string) => string,
  ): string {
    if (!state || state === "-") return state;
    const key = `netState.${state.toLowerCase()}`;
    const label = translate(key);
    return label === key ? state : label;
  }

  // Raw list → search → protocol → state → favorites-only → focused
  // process → sort → favorites pinned → group
  $: searchedConnections = filterConnections(
    connections,
    searchTerm,
    processNameByPid,
  );

  $: protocolFilteredConnections = searchedConnections.filter(
    (connection) =>
      protocolFilter === "all" ||
      connection.protocol.toLowerCase() === protocolFilter,
  );

  $: stateFilteredConnections = protocolFilteredConnections.filter(
    (connection) =>
      stateFilter === "all" || stateBucketOf(connection.state) === stateFilter,
  );

  // "With traffic only": drops idle connections (listeners, stagnant TCP)
  $: categoryFilteredConnections =
    categoryFilter === "all"
      ? stateFilteredConnections
      : stateFilteredConnections.filter(
          (connection) => categoryOf(connection) === categoryFilter,
        );

  $: trafficFilteredConnections = hideIdle
    ? categoryFilteredConnections.filter(
        (connection) =>
          connection.bytes_sent > 0 || connection.bytes_received > 0,
      )
    : categoryFilteredConnections;

  $: favoritesFilteredConnections = favoritesOnly
    ? trafficFilteredConnections.filter((connection) =>
        favoriteKeys.has(portKeyOf(connection)),
      )
    : trafficFilteredConnections;

  $: focusedConnections =
    focusedPid === null
      ? favoritesFilteredConnections
      : favoritesFilteredConnections.filter(
          (connection) => connection.pid === focusedPid,
        );

  $: filteredConnections = sortConnections(
    focusedConnections,
    sortField,
    sortDirection,
    processNameByPid,
    speedByKey,
  );

  // Favorites stay on top regardless of the active sort; the partition is
  // stable so rows keep their sorted order inside each half
  $: displayedConnections = partitionByFavorites(
    filteredConnections,
    favoriteKeys,
  );

  // The flat view renders in pages: an unfiltered list can hold thousands of
  // rows, each with several buttons, so only the first page mounts and a
  // trailing button reveals more. Search, filters, focus and sorting restart
  // from the first page; auto-refreshes keep the current page
  const FLAT_PAGE_SIZE = 500;
  let visibleFlatCount = FLAT_PAGE_SIZE;
  let lastFlatPageKey = "";
  $: visibleConnections = displayedConnections.slice(0, visibleFlatCount);
  $: {
    const flatPageKey = [
      searchTerm,
      protocolFilter,
      stateFilter,
      favoritesOnly,
      focusedPid,
      sortField,
      sortDirection,
    ].join("|");
    if (flatPageKey !== lastFlatPageKey) {
      lastFlatPageKey = flatPageKey;
      visibleFlatCount = FLAT_PAGE_SIZE;
    }
  }

  $: processGroups = groupByProcess(
    filteredConnections,
    processNameByPid,
    sortField,
    sortDirection,
    speedByKey,
  );

  $: displayedGroups = partitionGroupsByFavorites(processGroups, favoriteKeys);

  // Tree view: built from the focused (pre-sort) connections so toggling a
  // column sort does not rebuild it — the tree orders itself by aggregate
  // connection count instead. Rebuilt only when the connections, the
  // filters or the snapshot actually change (Svelte reactive chain).
  $: treeRoots = buildPortTree(
    focusedConnections,
    processPpidByPid,
    processNameByPid,
  );

  // Expansions are keyed by PID; drop the ones whose node left the tree so
  // exited processes don't accumulate stale state (a reused PID may still
  // inherit an expansion — the PID is the only identity available to key on)
  $: pruneExpandedTreeNodes(treeRoots);

  $: flatTreeRows = flattenPortTree(treeRoots, expandedTreeNodes);

  // --- Port deep-dive panel: resolved fresh on every snapshot so the panel
  // survives auto-refreshes and always shows current data ---
  $: detailConnection =
    detailKey === null
      ? null
      : (findConnectionByKey(filteredConnections, detailKey) ?? null);

  $: detailProcess = detailConnection
    ? (processByPid($processStore.processes, detailConnection.pid) ?? null)
    : null;

  $: detailSuspended = detailProcess
    ? $processStore.suspendedPids.has(detailProcess.pid)
    : false;

  // (Re)load the exe metadata once per PID; results come from the shared
  // path-keyed cache in processInfo.ts, so repeated opens are free
  $: if (detailProcess) {
    if (detailProcess.pid !== metadataPid) {
      metadataPid = detailProcess.pid;
      detailMetadata = null;
      detailError = null;
      loadDetailMetadata(detailProcess);
    }
  } else {
    detailMetadata = null;
    metadataPid = null;
    detailError = null;
  }

  $: detailChain = detailProcess
    ? buildAncestryChain(detailProcess, $processStore.processes)
    : null;

  // Opening the deep-dive panel is a user action on one listener: run the
  // role probe for it then (no-op when already probed/cached)
  $: if (detailConnection) {
    identifyPortRole(detailConnection);
  }

  /** Probe findings of the panel's listener, for the Server-header fact. */
  $: detailProbe = (void roleCacheVersion, detailConnection)
    ? (roleCache.get(roleKeyOf(detailConnection)) ?? null)
    : null;

  // --- Relations (witr-style): children and siblings from the snapshot ---
  $: detailChildren = detailProcess
    ? $processStore.processes.filter(
        (process) => process.ppid === detailProcess.pid,
      )
    : [];
  $: detailSiblings =
    detailProcess && detailProcess.ppid > 0
      ? $processStore.processes.filter(
          (process) =>
            process.ppid === detailProcess.ppid &&
            process.pid !== detailProcess.pid,
        )
      : [];

  // --- Integrity level: one backend query per PID, cached for the session ---
  let integrityByPid = new Map<number, string>();
  let detailIntegrity: string | null = null;

  $: if (detailProcess) {
    const pid = detailProcess.pid;
    if (!integrityByPid.has(pid)) {
      invoke<string>("get_process_integrity", { pid })
        .then((level) => {
          const next = new Map(integrityByPid);
          next.set(pid, level);
          integrityByPid = next;
          if (detailProcess?.pid === pid) {
            detailIntegrity = level;
          }
        })
        .catch(() => {});
    } else {
      detailIntegrity = integrityByPid.get(pid) ?? null;
    }
  } else {
    detailIntegrity = null;
  }

  $: detailPidConnections = detailConnection
    ? connectionsOfPid(connections, detailConnection.pid)
    : [];

  $: detailWarnings = detailConnection
    ? buildDetailWarnings(detailProcess, detailMetadata, detailPidConnections)
    : [];

  async function loadDetailMetadata(process: Process) {
    const metadata = await ensureProcessMetadata(process);
    // The panel may have moved to another process while loading
    if (metadataPid === process.pid) {
      detailMetadata = metadata;
    }
  }

  function findConnectionByKey(
    list: PortConnection[],
    key: string,
  ): PortConnection | null {
    return list.find((connection) => connectionKey(connection) === key) ?? null;
  }

  function processByPid(processes: Process[], pid: number): Process | null {
    return processes.find((process) => process.pid === pid) ?? null;
  }

  function connectionsOfPid(
    list: PortConnection[],
    pid: number,
  ): PortConnection[] {
    return list.filter((connection) => connection.pid === pid);
  }

  /**
   * Builds the ancestry tree of every process holding a (filtered)
   * connection. Nodes are created for connected processes and grown upward
   * along ppid chains, so ancestors without own connections stay visible as
   * structural nodes (bash → node → ZCode). Everything is Map-indexed and
   * single-pass O(n) over connections + involved processes; ppid cycles and
   * self-references are broken so the walk can never loop forever.
   * Returns the root nodes, siblings sorted by aggregate count descending.
   */
  function buildPortTree(
    list: PortConnection[],
    ppids: Map<number, number>,
    names: Map<number, string>,
  ): PortTreeNode[] {
    const nodes = new Map<number, PortTreeNode>();
    const ensureNode = (pid: number): PortTreeNode => {
      let node = nodes.get(pid);
      if (!node) {
        node = {
          pid,
          ppid: ppids.get(pid) ?? null,
          name: names.get(pid) ?? "-",
          connections: [],
          ports: [],
          ownCount: 0,
          totalCount: 0,
          parent: null,
          children: [],
        };
        nodes.set(pid, node);
      }
      return node;
    };

    // Deduplicate by 6-tuple first: the macOS lsof collector reports one row
    // per fd, so a dup'd fd yields two identical records, and the tree's
    // keyed each must not see the same connection key twice
    const seenKeys = new Set<string>();
    const unique = list.filter((connection) => {
      const key = connectionKey(connection);
      if (seenKeys.has(key)) return false;
      seenKeys.add(key);
      return true;
    });

    // Attach every connection; PIDs missing from the snapshot (exited or
    // unreadable processes) get synthetic root nodes so nothing is lost
    for (const connection of unique) {
      ensureNode(connection.pid).connections.push(connection);
    }

    // Grow the ancestor chain of every connected process. The walk stops at
    // ppid 0 (no parent, same semantics as buildAncestryChain), snapshot
    // boundaries, self-references, cycles (visited set) and at nodes whose
    // own chain is already complete (memo), keeping it O(n).
    for (const start of nodes.values()) {
      if (start.connections.length === 0) continue;
      const visited = new Set<number>([start.pid]);
      let current = start;
      for (;;) {
        const ppid = current.ppid;
        if (
          !ppid ||
          ppid === current.pid ||
          visited.has(ppid) ||
          !ppids.has(ppid) ||
          nodes.has(ppid)
        ) {
          break;
        }
        const parent = ensureNode(ppid);
        visited.add(ppid);
        current = parent;
      }
    }

    // Link each node to its parent node; unresolved parents (unknown,
    // self-referenced or ppid 0) stay roots
    for (const node of nodes.values()) {
      if (!node.ppid || node.ppid === node.pid) continue;
      const parent = nodes.get(node.ppid);
      if (parent && parent !== node) node.parent = parent;
    }

    // Safety net for ppid cycles: any node whose ancestor walk re-enters
    // its own path is cut loose and re-rooted so it still renders (and its
    // subtree keeps its aggregates) instead of disappearing or looping
    const state = new Map<PortTreeNode, 1 | 2>(); // 1 = on path, 2 = resolved
    for (const node of nodes.values()) {
      if (state.has(node)) continue;
      const path: PortTreeNode[] = [];
      let current: PortTreeNode | null = node;
      while (current && !state.has(current)) {
        state.set(current, 1);
        path.push(current);
        current = current.parent;
      }
      // Walk ended at a root (null) or a resolved node → path is fine;
      // ended on the path itself → cycle members get re-rooted
      const cycleStart =
        current && state.get(current) === 1 ? path.indexOf(current) : -1;
      for (let i = 0; i < path.length; i++) {
        if (cycleStart >= 0 && i >= cycleStart) {
          path[i].parent = null;
        }
        state.set(path[i], 2);
      }
    }

    // Children lists and roots from the final parent links
    const roots: PortTreeNode[] = [];
    for (const node of nodes.values()) {
      if (node.parent) node.parent.children.push(node);
      else roots.push(node);
    }

    // Aggregate connection counts bottom-up, then prune the per-node port
    // list; the forest is acyclic by construction, so plain recursion
    const finalize = (node: PortTreeNode): number => {
      let total = node.connections.length;
      for (const child of node.children) total += finalize(child);
      node.ownCount = node.connections.length;
      node.connections.sort((a, b) => a.local_port - b.local_port);
      node.ports = Array.from(
        new Set(node.connections.map((c) => c.local_port)),
      ).sort((a, b) => a - b);
      node.totalCount = total;
      return total;
    };
    for (const root of roots) finalize(root);

    // Tree view ordering: aggregate connection count descending, own count
    // and PID as deterministic tie-breakers
    const byTotalDesc = (a: PortTreeNode, b: PortTreeNode) =>
      b.totalCount - a.totalCount || b.ownCount - a.ownCount || a.pid - b.pid;
    const sortLevel = (level: PortTreeNode[]) => {
      level.sort(byTotalDesc);
      for (const node of level) sortLevel(node.children);
    };
    sortLevel(roots);

    return roots;
  }

  /**
   * Flattens the tree into renderable rows: a node row per process followed
   * (when expanded) by its connection detail rows and its children, depth
   * first. Collapsed nodes contribute just their own row, so a folded tree
   * renders one row per top-level process regardless of scale.
   */
  function flattenPortTree(
    roots: PortTreeNode[],
    expanded: Set<number>,
  ): PortTreeRow[] {
    const rows: PortTreeRow[] = [];
    const walk = (node: PortTreeNode, depth: number) => {
      rows.push({ kind: "node", depth, node });
      if (!expanded.has(node.pid)) return;
      for (const connection of node.connections) {
        rows.push({ kind: "connection", depth: depth + 1, connection });
      }
      for (const child of node.children) walk(child, depth + 1);
    };
    for (const root of roots) walk(root, 0);
    return rows;
  }

  /** Directionless comparator for the sortable columns; call sites apply
   * the sort factor so the flat view and the grouped view's in-group rows
   * share the exact same ordering semantics. */
  function compareByField(
    a: PortConnection,
    b: PortConnection,
    field: SortField,
    names: Map<number, string>,
    speeds: Record<string, { down: number; up: number }>,
  ): number {
    const speedOf = (connection: PortConnection, key: "down" | "up") =>
      speeds[connectionKey(connection)]?.[key] ?? 0;
    switch (field) {
      case "local_port":
        return a.local_port - b.local_port;
      case "remote_addr":
        return (
          a.remote_addr.localeCompare(b.remote_addr) ||
          a.remote_port - b.remote_port
        );
      case "state":
        return a.state.localeCompare(b.state);
      case "pid":
        return a.pid - b.pid;
      case "process":
        return (names.get(a.pid) ?? "").localeCompare(names.get(b.pid) ?? "");
      case "download":
        return speedOf(a, "down") - speedOf(b, "down");
      case "upload":
        return speedOf(a, "up") - speedOf(b, "up");
      default:
        return 0;
    }
  }

  function sortConnections(
    list: PortConnection[],
    field: SortField | null,
    direction: SortDirection,
    names: Map<number, string>,
    speeds: Record<string, { down: number; up: number }>,
  ): PortConnection[] {
    if (field === null) return list;
    const factor = direction === "asc" ? 1 : -1;
    return [...list].sort(
      (a, b) => compareByField(a, b, field, names, speeds) * factor,
    );
  }

  function groupByProcess(
    list: PortConnection[],
    names: Map<number, string>,
    field: SortField | null,
    direction: SortDirection,
    speeds: Record<string, { down: number; up: number }>,
  ): ProcessPortGroup[] {
    const groups = new Map<number, ProcessPortGroup>();
    for (const connection of list) {
      let group = groups.get(connection.pid);
      if (!group) {
        group = {
          pid: connection.pid,
          name: names.get(connection.pid) ?? "-",
          count: 0,
          ports: [],
          connections: [],
        };
        groups.set(connection.pid, group);
      }
      group.connections.push(connection);
    }
    const factor = direction === "asc" ? 1 : -1;
    for (const group of groups.values()) {
      // Rows inside a group follow the active column sort (same semantics
      // as the flat view — the header sort must not dead-end here); with
      // no sort active, fall back to the historical local-port order
      group.connections.sort((a, b) =>
        field === null
          ? a.local_port - b.local_port
          : compareByField(a, b, field, names, speeds) * factor,
      );
      group.ports = Array.from(
        new Set(group.connections.map((c) => c.local_port)),
      ).sort((a, b) => a - b);
      group.count = group.connections.length;
    }
    return Array.from(groups.values());
  }

  function toggleSort(field: SortField) {
    if (sortField === field) {
      sortDirection = sortDirection === "asc" ? "desc" : "asc";
    } else {
      sortField = field;
      sortDirection = "asc";
    }
  }

  function toggleGroup(pid: number) {
    const next = new Set(expandedGroups);
    if (next.has(pid)) {
      next.delete(pid);
    } else {
      next.add(pid);
    }
    expandedGroups = next;
  }

  function toggleTreeNode(pid: number) {
    const next = new Set(expandedTreeNodes);
    if (next.has(pid)) {
      next.delete(pid);
    } else {
      next.add(pid);
    }
    expandedTreeNodes = next;
  }

  /** Collapses every expanded group and tree node at once (grouped and
   * tree views); no counterpart expand-all on purpose. */
  function collapseAll() {
    expandedGroups = new Set();
    expandedTreeNodes = new Set();
  }

  /** Removes expansion state whose node no longer exists in the tree, so
   * it doesn't survive the process that owned it. */
  function pruneExpandedTreeNodes(roots: PortTreeNode[]) {
    if (expandedTreeNodes.size === 0) return;
    const live = new Set<number>();
    const walk = (node: PortTreeNode) => {
      live.add(node.pid);
      for (const child of node.children) walk(child);
    };
    for (const root of roots) walk(root);
    let pruned = false;
    const next = new Set<number>();
    for (const pid of expandedTreeNodes) {
      if (live.has(pid)) {
        next.add(pid);
      } else {
        pruned = true;
      }
    }
    if (pruned) expandedTreeNodes = next;
  }

  function focusProcess(pid: number) {
    focusedPid = pid;
    focusedName = processNameByPid.get(pid) ?? "-";
  }

  function clearFocus() {
    focusedPid = null;
    focusedName = "";
  }

  function sortIndicator(field: SortField): string {
    if (sortField !== field) return "↕";
    return sortDirection === "asc" ? "↑" : "↓";
  }

  function sortAria(field: SortField): "ascending" | "descending" | "none" {
    if (sortField !== field) return "none";
    return sortDirection === "asc" ? "ascending" : "descending";
  }
</script>

<Modal {show} title={$t("ports.title")} maxWidth="1020px" {onClose}>
  <div class="ports-content">
    <div class="ports-toolbar">
      <input
        class="ports-search"
        type="text"
        placeholder={$t("ports.searchPlaceholder")}
        bind:value={searchTerm}
      />
      <div class="view-toggle" role="group" aria-label={$t("tools.viewMode")}>
        <button
          class:active={viewMode === "flat"}
          on:click={() => setViewMode("flat")}
          title={$t("ports.flatList")}
          aria-label={$t("ports.flatList")}
        >
          <Fa icon={faList} />
        </button>
        <button
          class:active={viewMode === "grouped"}
          on:click={() => setViewMode("grouped")}
          title={$t("ports.groupByProcess")}
          aria-label={$t("ports.groupByProcess")}
        >
          <Fa icon={faLayerGroup} />
        </button>
        <button
          class:active={viewMode === "tree"}
          on:click={() => setViewMode("tree")}
          title={$t("ports.groupByTree")}
          aria-label={$t("ports.groupByTree")}
        >
          <Fa icon={faSitemap} />
        </button>
      </div>
      {#if viewMode !== "flat"}
        <button
          class="collapse-toggle"
          on:click={collapseAll}
          title={$t("ports.collapseAll")}
          aria-label={$t("ports.collapseAll")}
        >
          <Fa icon={faCompressArrowsAlt} />
        </button>
      {/if}
      <button
        class="collapse-toggle"
        on:click={() => loadConnections(true)}
        disabled={isLoading && manualLoad}
        title={$t("ports.refresh")}
        aria-label={$t("ports.refreshAria")}
      >
        {#if isLoading && manualLoad}
          <span class="spinner"></span>
        {:else}
          <Fa icon={faRefresh} />
        {/if}
      </button>
    </div>

    <div class="ports-chips">
      <span class="chip-label">{$t("ports.protocolLabel")}</span>
      <button
        class="chip"
        class:active={protocolFilter === "all"}
        on:click={() => (protocolFilter = "all")}
      >
        {$t("ports.filterAll")}
      </button>
      <button
        class="chip"
        class:active={protocolFilter === "tcp"}
        on:click={() => (protocolFilter = "tcp")}
      >
        {$t("ports.filterTcp")}
      </button>
      <button
        class="chip"
        class:active={protocolFilter === "udp"}
        on:click={() => (protocolFilter = "udp")}
      >
        {$t("ports.filterUdp")}
      </button>
      <span class="chip-divider"></span>
      <span class="chip-label">{$t("ports.stateLabel")}</span>
      <button
        class="chip"
        class:active={stateFilter === "all"}
        on:click={() => (stateFilter = "all")}
      >
        {$t("ports.stateAll")}
      </button>
      <button
        class="chip"
        class:active={stateFilter === "listen"}
        on:click={() => (stateFilter = "listen")}
      >
        {$t("ports.stateListen")}
      </button>
      <button
        class="chip"
        class:active={stateFilter === "established"}
        on:click={() => (stateFilter = "established")}
      >
        {$t("ports.stateEstablished")}
      </button>
      <button
        class="chip"
        class:active={stateFilter === "other"}
        on:click={() => (stateFilter = "other")}
      >
        {$t("ports.stateOther")}
      </button>
      <span class="chip-divider"></span>
      <span class="chip-label">{$t("ports.categoryLabel")}</span>
      <button
        class="chip"
        class:active={categoryFilter === "all"}
        on:click={() => (categoryFilter = "all")}
      >
        {$t("ports.filterAll")}
      </button>
      {#each PORT_CATEGORIES as category (category.key)}
        <button
          class="chip"
          class:active={categoryFilter === category.key}
          on:click={() => (categoryFilter = category.key)}
        >
          {$t(`ports.cat.${category.key}`)}
        </button>
      {/each}
      <span class="chip-divider"></span>
      <button
        class="chip star-chip"
        class:active={favoritesOnly}
        on:click={() => (favoritesOnly = !favoritesOnly)}
        title={$t("ports.favoritesOnly")}
        aria-label={$t("ports.favoritesOnly")}
      >
        <Fa icon={faStar} />
      </button>
      {#if showSpeedCols}
        <button
          class="chip"
          class:active={hideIdle}
          on:click={() => (hideIdle = !hideIdle)}
          title={$t("ports.hideIdle")}
          aria-label={$t("ports.hideIdle")}
        >
          <Fa icon={faChartLine} />
        </button>
        <span class="chip-divider"></span>
        <button
          class="chip star-chip"
          class:active={realtime}
          on:click={() => (realtime = !realtime)}
          title={$t("ports.realtimeTitle")}
          aria-pressed={realtime}
          aria-label={$t("ports.realtimeTitle")}
        >
          <Fa icon={faBolt} />
        </button>
      {/if}
    </div>

    {#if focusedPid !== null}
      <div class="focus-row">
        <span class="focus-chip">
          <Fa icon={faCrosshairs} />
          <span>{focusedName}</span>
          <span class="focus-pid">{$t("modal.pid", { pid: focusedPid })}</span>
          <button
            class="focus-clear"
            on:click={clearFocus}
            title={$t("ports.clearFocus")}
            aria-label={$t("ports.clearFocus")}
          >
            <Fa icon={faXmark} />
          </button>
        </span>
      </div>
    {/if}

    {#if error}
      <div class="ports-error">{error}</div>
    {/if}

    {#if isLoading && connections.length === 0}
      <div class="ports-status">
        <div class="spinner"></div>
      </div>
    {:else if connections.length === 0}
      <div class="ports-status">{$t("ports.empty")}</div>
    {:else}
      <div class="count-row">
        {$t("ports.count", {
          shown: filteredConnections.length,
          total: connections.length,
        })}
        {#if !$isElevated}
          <span class="traffic-hint">{$t("ports.trafficAdminHint")}</span>
        {/if}
      </div>

      {#if watchedPorts.length > 0}
        <div class="watched-row">
          <span class="watched-label">{$t("ports.watchedPorts")}</span>
          {#each [...watchedPorts].sort((a, b) => a - b) as port (port)}
            <button
              class="watch-chip"
              class:live={liveListenPorts.has(port)}
              on:click={() => togglePortWatch(port)}
              title={$t("ports.watchRemove")}
            >
              <Fa icon={faBell} />
              {port}
              <span class="watch-x">×</span>
            </button>
          {/each}
        </div>
      {/if}

      <!-- Shared action cluster of one connection row: favorite star, deep-dive
           panel toggle, process focus, quick kill and close connection -->
      {#snippet rowActions(connection: PortConnection)}
        {@const favorited = favoriteKeys.has(portKeyOf(connection))}
        {@const portActionKey = connectionKey(connection)}
        <div class="row-actions">
          <button
            class="focus-btn star-btn"
            class:favorited
            on:click={() => togglePortFavorite(portKeyOf(connection))}
            title={favorited
              ? $t("ports.favoriteRemove")
              : $t("ports.favoriteAdd")}
            aria-label={favorited
              ? $t("ports.favoriteRemove")
              : $t("ports.favoriteAdd")}
          >
            <Fa icon={faStar} />
          </button>
          {#if connection.state === "LISTEN"}
            {@const watched = watchedPorts.includes(connection.local_port)}
            <button
              class="focus-btn watch-btn"
              class:active={watched}
              on:click={() => togglePortWatch(connection.local_port)}
              title={watched ? $t("ports.watchRemove") : $t("ports.watchAdd")}
              aria-label={watched
                ? $t("ports.watchRemove")
                : $t("ports.watchAdd")}
            >
              <Fa icon={faBell} />
            </button>
          {/if}
          <button
            class="focus-btn"
            class:active={detailConnection === connection}
            on:click={() => toggleDetail(connection)}
            title={$t("ports.rowDetails")}
            aria-label={$t("ports.rowDetails")}
          >
            <Fa icon={faCircleInfo} />
          </button>
          {#if isListenablePort(connection)}
            {@const role = portRoleOf(connection)}
            {#if role?.open}
              <button
                class="focus-btn"
                on:click={() => openPortInBrowser(connection)}
                title={$t("ports.openInBrowser")}
                aria-label={$t("ports.openInBrowser")}
              >
                <Fa icon={faUpRightFromSquare} />
              </button>
            {/if}
            <button
              class="focus-btn"
              class:copied={copiedPortKeys.has(portActionKey)}
              on:click={() => copyPortAddress(connection)}
              title={$t("ports.copyAddress")}
              aria-label={$t("ports.copyAddress")}
            >
              <Fa icon={copiedPortKeys.has(portActionKey) ? faCheck : faCopy} />
            </button>
            <button
              class="focus-btn"
              class:active={identifiedKeys.has(roleKeyOf(connection))}
              disabled={probingKeys.has(roleKeyOf(connection))}
              on:click={() => identifyPortRole(connection)}
              title={$t("ports.identify")}
              aria-label={$t("ports.identify")}
            >
              <Fa
                icon={probingKeys.has(roleKeyOf(connection))
                  ? faSpinner
                  : faStethoscope}
                spin={probingKeys.has(roleKeyOf(connection))}
              />
            </button>
          {/if}
          <button
            class="focus-btn"
            class:active={focusedPid === connection.pid}
            on:click={() => focusProcess(connection.pid)}
            title={$t("ports.focusProcess")}
            aria-label={$t("ports.focusProcess")}
          >
            <Fa icon={faCrosshairs} />
          </button>
          {#if canKillProcess(connection.pid)}
            <button
              class="focus-btn danger"
              on:click={() =>
                confirmKillProcess(
                  connection.pid,
                  processNameByPid.get(connection.pid) ?? "-",
                )}
              title={$t("ports.killProcess")}
              aria-label={$t("ports.killProcess")}
            >
              <Fa icon={faBan} />
            </button>
          {/if}
          {#if canCloseConnection(connection)}
            <button
              class="focus-btn danger"
              on:click={() => confirmCloseConnection(connection)}
              title={$t("ports.closeConnection")}
              aria-label={$t("ports.closeConnection")}
            >
              <Fa icon={faXmark} />
            </button>
          {/if}
        </div>
        {#if portActionNotice && portActionNotice.key === portActionKey}
          <span class="row-notice">{portActionNotice.message}</span>
        {/if}
      {/snippet}

      <!-- Per-connection traffic speeds (deltas of the backend's cumulative
           TCP byte counters); rendered as two cells in every view -->
      {#snippet trafficCells(connection: PortConnection)}
        {#if showSpeedCols}
          {@const speed = connectionSpeed(connection)}
          <td class="mono speed">{speedText(speed.down)}</td>
          <td class="mono speed">{speedText(speed.up)}</td>
        {/if}
      {/snippet}

      <!-- Local endpoint with a category tag for well-known ports, the
           probed service role and the security-relevant bind scope -->
      {#snippet localPortCell(connection: PortConnection)}
        {@const category = categoryOf(connection)}
        {@const role = portRoleOf(connection)}
        {@const bindScope = bindScopeOf(connection)}
        <td class="mono">
          {connection.local_addr}:{connection.local_port}
          {#if category}
            <span class="port-cat cat-{category}"
              >{$t(`ports.cat.${category}`)}</span
            >
          {/if}
          {#if role?.tag}
            <span class="role-tag role-{role.tag}"
              >{$t(`ports.role.${role.tag}`)}</span
            >
          {/if}
          {#if bindScope}
            <span class="bind-scope">{$t(bindScope)}</span>
          {/if}
        </td>
      {/snippet}

      <!-- In-modal deep-dive panel rendered under the selected connection row:
           warnings, ancestry chain ("who launched it"), connection and process
           facts, command line and actions -->
      {#snippet detailRow(connection: PortConnection)}
        {#if detailConnection === connection}
          <tr
            class="detail-row"
            in:fly={{ y: -4, duration: 150 }}
            out:fade={{ duration: 100 }}
          >
            <td colspan={showSpeedCols ? 9 : 7}>
              <div class="detail-panel">
                {#if detailWarnings.length > 0 || detailMetadata?.binary_missing}
                  <div class="detail-warnings">
                    {#each detailWarnings as warning (warning.key)}
                      <span class="warn-badge {warning.tone}">
                        <Fa icon={faTriangleExclamation} />
                        {$t(warning.key)}
                      </span>
                    {/each}
                    {#if detailMetadata?.binary_missing}
                      <span class="warn-badge red">
                        <Fa icon={faTriangleExclamation} />
                        {$t("ports.binaryMissing")}
                      </span>
                    {/if}
                  </div>
                {/if}

                <div class="detail-chain-block">
                  <span class="chain-label">{$t("ports.chainLabel")}</span>
                  <div class="detail-chain">
                    {#if detailChain}
                      {#each detailChain.segments as segment, i (segment.pid)}
                        {#if i > 0}
                          <span class="chain-arrow">→</span>
                        {/if}
                        <span
                          class="chain-seg"
                          class:subject={segment.isSubject}
                          title={segment.command}
                        >
                          <span class="chain-name">{segment.name}</span>
                          <span class="chain-pid">({segment.pid})</span>
                        </span>
                      {/each}
                      {#if detailChain.broken}
                        <span class="chain-broken"
                          >{$t("ports.chainBroken")}</span
                        >
                      {/if}
                    {:else}
                      <span class="chain-broken">{$t("ports.chainBroken")}</span
                      >
                    {/if}
                  </div>
                </div>

                <div class="detail-facts">
                  <div class="fact">
                    <span class="fact-label">{$t("ports.protocol")}</span>
                    <span class="fact-value">{connection.protocol}</span>
                  </div>
                  <div class="fact">
                    <span class="fact-label">{$t("ports.localAddress")}</span>
                    <span class="fact-value mono"
                      >{connection.local_addr}:{connection.local_port}</span
                    >
                  </div>
                  <div class="fact">
                    <span class="fact-label">{$t("ports.remoteAddress")}</span>
                    <span class="fact-value mono">
                      {#if connection.remote_addr}
                        {connection.remote_addr}:{connection.remote_port}
                      {:else}
                        -
                      {/if}
                    </span>
                  </div>
                  <div class="fact">
                    <span class="fact-label">{$t("ports.state")}</span>
                    <span class="fact-value mono">{connection.state}</span>
                  </div>
                  {#if connection.bytes_sent > 0 || connection.bytes_received > 0}
                    <div class="fact">
                      <span class="fact-label">{$t("ports.received")}</span>
                      <span class="fact-value mono"
                        >{formatBytes(connection.bytes_received)}</span
                      >
                    </div>
                    <div class="fact">
                      <span class="fact-label">{$t("ports.sent")}</span>
                      <span class="fact-value mono"
                        >{formatBytes(connection.bytes_sent)}</span
                      >
                    </div>
                  {/if}
                </div>

                <div class="detail-facts">
                  <div class="fact">
                    <span class="fact-label">{$t("ports.process")}</span>
                    <span class="fact-value">
                      {detailProcess?.name ?? "-"}
                      {#if detailProcess}
                        <span class="fact-pid"
                          >{$t("modal.pid", { pid: detailProcess.pid })}</span
                        >
                      {/if}
                    </span>
                  </div>
                  <div class="fact">
                    <span class="fact-label">{$t("ports.fUser")}</span>
                    <span class="fact-value">{detailProcess?.user ?? "-"}</span>
                  </div>
                  <div class="fact">
                    <span class="fact-label">{$t("ports.fStartTime")}</span>
                    <span class="fact-value"
                      >{detailProcess
                        ? formatDate(detailProcess.start_time)
                        : "-"}</span
                    >
                  </div>
                  <div class="fact">
                    <span class="fact-label">{$t("ports.fMemory")}</span>
                    <span class="fact-value"
                      >{detailProcess
                        ? formatBytes(detailProcess.memory_usage)
                        : "-"}</span
                    >
                  </div>
                  <div class="fact">
                    <span class="fact-label">{$t("ports.fRunTime")}</span>
                    <span class="fact-value"
                      >{detailProcess
                        ? formatUptime(detailProcess.run_time)
                        : "-"}</span
                    >
                  </div>
                  <div class="fact">
                    <span class="fact-label">{$t("ports.fCpu")}</span>
                    <span class="fact-value mono"
                      >{detailProcess
                        ? formatPercentage(detailProcess.cpu_usage)
                        : "-"}</span
                    >
                  </div>
                  <div class="fact">
                    <span class="fact-label">{$t("ports.integrity")}</span>
                    <span class="fact-value">
                      {#if detailIntegrity}
                        <span
                          class="integrity-badge integrity-{detailIntegrity}"
                          >{$t(`integrity.${detailIntegrity}`)}</span
                        >
                      {:else}
                        -
                      {/if}
                    </span>
                  </div>
                  {#if detailMetadata && detailMetadata.signed !== null}
                    <div class="fact">
                      <span class="fact-label">{$t("ports.fSignature")}</span>
                      <span class="fact-value">
                        <span
                          class="sig-badge"
                          class:signed={detailMetadata.signed}
                          class:unsigned={!detailMetadata.signed}
                        >
                          <Fa
                            icon={detailMetadata.signed
                              ? faCircleCheck
                              : faTriangleExclamation}
                          />
                          {$t(
                            detailMetadata.signed
                              ? "ports.signed"
                              : "ports.unsigned",
                          )}
                        </span>
                      </span>
                    </div>
                  {/if}
                  {#if detailProbe?.http?.server}
                    <div class="fact">
                      <span class="fact-label">{$t("ports.fServer")}</span>
                      <span class="fact-value mono"
                        >{detailProbe.http.server}</span
                      >
                    </div>
                  {/if}
                  {#if detailMetadata?.company}
                    <div class="fact">
                      <span class="fact-label">{$t("ports.fCompany")}</span>
                      <span class="fact-value">{detailMetadata.company}</span>
                    </div>
                  {/if}
                  {#if detailMetadata?.version}
                    <div class="fact">
                      <span class="fact-label">{$t("ports.fVersion")}</span>
                      <span class="fact-value mono"
                        >{detailMetadata.version}</span
                      >
                    </div>
                  {/if}
                  {#if detailMetadata?.description}
                    <div class="fact">
                      <span class="fact-label">{$t("ports.fDescription")}</span>
                      <span class="fact-value"
                        >{detailMetadata.description}</span
                      >
                    </div>
                  {/if}
                </div>

                {#if detailChildren.length > 0 || detailSiblings.length > 0}
                  <div class="detail-relations">
                    <div class="relation-row">
                      <span class="relation-label"
                        >{$t("ports.children", {
                          count: detailChildren.length,
                        })}</span
                      >
                      <div class="relation-chips">
                        {#each detailChildren.slice(0, 6) as child (child.pid)}
                          <button
                            class="relation-chip"
                            on:click={() => openProcessDetails(child.pid)}
                            title={$t("modal.pid", { pid: child.pid })}
                          >
                            {child.name}
                          </button>
                        {/each}
                        {#if detailChildren.length > 6}
                          <span class="relation-more"
                            >+{detailChildren.length - 6}</span
                          >
                        {/if}
                        {#if detailChildren.length === 0}
                          <span class="relation-none">-</span>
                        {/if}
                      </div>
                    </div>
                    {#if detailSiblings.length > 0}
                      <div class="relation-row">
                        <span class="relation-label"
                          >{$t("ports.siblings", {
                            count: detailSiblings.length,
                          })}</span
                        >
                        <div class="relation-chips">
                          {#each detailSiblings.slice(0, 6) as sibling (sibling.pid)}
                            <button
                              class="relation-chip"
                              on:click={() => openProcessDetails(sibling.pid)}
                              title={$t("modal.pid", { pid: sibling.pid })}
                            >
                              {sibling.name}
                            </button>
                          {/each}
                          {#if detailSiblings.length > 6}
                            <span class="relation-more"
                              >+{detailSiblings.length - 6}</span
                            >
                          {/if}
                        </div>
                      </div>
                    {/if}
                  </div>
                {/if}

                {#if detailProcess?.command}
                  <div class="detail-command">{detailProcess.command}</div>
                {/if}

                {#if detailError}
                  <div class="ports-error">{detailError}</div>
                {/if}

                <div class="detail-actions">
                  {#if detailProcess}
                    {@const process = detailProcess}
                    <button
                      class="btn-danger"
                      on:click={() =>
                        confirmKillProcess(process.pid, process.name)}
                    >
                      <Fa icon={faBan} />
                      <span>{$t("ports.killProcess")}</span>
                    </button>
                    <button
                      class="btn-secondary"
                      on:click={toggleDetailSuspend}
                      disabled={isTogglingSuspend}
                    >
                      <Fa icon={detailSuspended ? faPlay : faPause} />
                      <span
                        >{detailSuspended
                          ? $t("action.resumeProcess")
                          : $t("action.suspendProcess")}</span
                      >
                    </button>
                    <button
                      class="btn-secondary"
                      on:click={() => openProcessDetails(process.pid)}
                    >
                      <Fa icon={faCrosshairs} />
                      <span>{$t("ports.openDetails")}</span>
                    </button>
                  {/if}
                  {#if canCloseConnection(connection)}
                    <button
                      class="btn-secondary"
                      on:click={() => confirmCloseConnection(connection)}
                    >
                      <Fa icon={faXmark} />
                      <span>{$t("ports.closeConnection")}</span>
                    </button>
                  {/if}
                </div>
              </div>
            </td>
          </tr>
        {/if}
      {/snippet}

      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>{$t("ports.protocol")}</th>
              <th aria-sort={sortAria("local_port")}>
                <button
                  class="th-sort"
                  disabled={viewMode === "tree"}
                  on:click={() => toggleSort("local_port")}
                >
                  {$t("ports.localAddress")}
                  <span
                    class="sort-indicator"
                    class:active={sortField === "local_port"}
                  >
                    {sortIndicator("local_port")}
                  </span>
                </button>
              </th>
              <th aria-sort={sortAria("remote_addr")}>
                <button
                  class="th-sort"
                  disabled={viewMode === "tree"}
                  on:click={() => toggleSort("remote_addr")}
                >
                  {$t("ports.remoteAddress")}
                  <span
                    class="sort-indicator"
                    class:active={sortField === "remote_addr"}
                  >
                    {sortIndicator("remote_addr")}
                  </span>
                </button>
              </th>
              <th aria-sort={sortAria("state")}>
                <button
                  class="th-sort"
                  disabled={viewMode === "tree"}
                  on:click={() => toggleSort("state")}
                >
                  {$t("ports.state")}
                  <span
                    class="sort-indicator"
                    class:active={sortField === "state"}
                  >
                    {sortIndicator("state")}
                  </span>
                </button>
              </th>
              <th aria-sort={sortAria("pid")}>
                <button
                  class="th-sort"
                  disabled={viewMode === "tree"}
                  on:click={() => toggleSort("pid")}
                >
                  {$t("ports.pid")}
                  <span
                    class="sort-indicator"
                    class:active={sortField === "pid"}
                  >
                    {sortIndicator("pid")}
                  </span>
                </button>
              </th>
              <th aria-sort={sortAria("process")}>
                <button
                  class="th-sort"
                  disabled={viewMode === "tree"}
                  on:click={() => toggleSort("process")}
                >
                  {$t("ports.process")}
                  <span
                    class="sort-indicator"
                    class:active={sortField === "process"}
                  >
                    {sortIndicator("process")}
                  </span>
                </button>
              </th>
              {#if showSpeedCols}
                <th aria-sort={sortAria("download")}>
                  <button
                    class="th-sort"
                    disabled={viewMode === "tree"}
                    on:click={() => toggleSort("download")}
                  >
                    {$t("ports.downloadSpeed")}
                    <span
                      class="sort-indicator"
                      class:active={sortField === "download"}
                    >
                      {sortIndicator("download")}
                    </span>
                  </button>
                </th>
                <th aria-sort={sortAria("upload")}>
                  <button
                    class="th-sort"
                    disabled={viewMode === "tree"}
                    on:click={() => toggleSort("upload")}
                  >
                    {$t("ports.uploadSpeed")}
                    <span
                      class="sort-indicator"
                      class:active={sortField === "upload"}
                    >
                      {sortIndicator("upload")}
                    </span>
                  </button>
                </th>
              {/if}
              <th class="actions-col"></th>
            </tr>
          </thead>
          <tbody>
            {#if viewMode === "flat"}
              {#each visibleConnections as connection}
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <tr
                  on:dblclick={(event) => handleRowDblClick(event, connection)}
                >
                  <td class="protocol">{connection.protocol}</td>
                  {@render localPortCell(connection)}
                  <td class="mono">
                    {#if connection.remote_addr}
                      {connection.remote_addr}:{connection.remote_port}
                    {:else}
                      -
                    {/if}
                  </td>
                  <td class="mono" title={connection.state}
                    >{localizedState(connection.state, $t)}</td
                  >
                  <td class="mono">{connection.pid || "-"}</td>
                  <td>
                    <button
                      class="process-link"
                      on:click={() => focusProcess(connection.pid)}
                      title={$t("ports.focusProcess")}
                    >
                      {processNameByPid.get(connection.pid) ?? "-"}
                    </button>
                  </td>
                  {@render trafficCells(connection)}
                  <td class="actions-col">
                    {@render rowActions(connection)}
                  </td>
                </tr>
                {@render detailRow(connection)}
              {/each}
              {#if displayedConnections.length > visibleConnections.length}
                <tr class="load-more-row">
                  <td colspan={showSpeedCols ? 9 : 7}>
                    <button
                      class="load-more"
                      on:click={() => (visibleFlatCount += FLAT_PAGE_SIZE)}
                    >
                      {$t("ports.showMore", {
                        count:
                          displayedConnections.length -
                          visibleConnections.length,
                      })}
                    </button>
                  </td>
                </tr>
              {/if}
              {#if displayedConnections.length === 0}
                <tr>
                  <td class="empty-row" colspan={showSpeedCols ? 9 : 7}
                    >{$t("ports.noResults")}</td
                  >
                </tr>
              {/if}
            {:else if viewMode === "grouped"}
              {#each displayedGroups as group (group.pid)}
                <tr class="group-row">
                  <td colspan={showSpeedCols ? 9 : 7}>
                    <div class="group-head-row">
                      <button
                        class="group-head"
                        on:click={() => toggleGroup(group.pid)}
                        aria-expanded={expandedGroups.has(group.pid)}
                      >
                        <span class="group-caret">
                          <Fa icon={faCaretRight} />
                        </span>
                        <!-- svelte-ignore a11y_no_static_element_interactions -->
                        <span
                          class="group-name"
                          on:dblclick={() => openProcessDetails(group.pid)}
                          title={$t("ports.openDetails")}>{group.name}</span
                        >
                        {#if group.pid}
                          <span class="group-pid"
                            >{$t("ports.pid")} {group.pid}</span
                          >
                        {/if}
                      </button>
                      {#if group.pid}
                        {@const favorited = group.connections.some(
                          (connection) =>
                            favoriteKeys.has(portKeyOf(connection)),
                        )}
                        <div class="head-actions">
                          <button
                            class="focus-btn star-btn"
                            class:favorited
                            on:click={() =>
                              toggleGroupFavorites(group.connections)}
                            title={favorited
                              ? $t("ports.favoriteRemove")
                              : $t("ports.favoriteAdd")}
                            aria-label={favorited
                              ? $t("ports.favoriteRemove")
                              : $t("ports.favoriteAdd")}
                          >
                            <Fa icon={faStar} />
                          </button>
                          {#if canKillProcess(group.pid)}
                            <button
                              class="focus-btn danger"
                              on:click={() =>
                                confirmKillProcess(group.pid, group.name)}
                              title={$t("ports.killProcess")}
                              aria-label={$t("ports.killProcess")}
                            >
                              <Fa icon={faBan} />
                            </button>
                          {/if}
                        </div>
                      {/if}
                    </div>
                  </td>
                </tr>
                {#if expandedGroups.has(group.pid)}
                  {#each group.connections as connection}
                    <!-- svelte-ignore a11y_no_static_element_interactions -->
                    <tr
                      class="child-row"
                      on:dblclick={(event) =>
                        handleRowDblClick(event, connection)}
                      in:fly={{ y: -4, duration: 150 }}
                      out:fade={{ duration: 100 }}
                    >
                      <td class="protocol">{connection.protocol}</td>
                      {@render localPortCell(connection)}
                      <td class="mono">
                        {#if connection.remote_addr}
                          {connection.remote_addr}:{connection.remote_port}
                        {:else}
                          -
                        {/if}
                      </td>
                      <td class="mono" title={connection.state}
                        >{localizedState(connection.state, $t)}</td
                      >
                      <td class="mono">{connection.pid || "-"}</td>
                      <td>
                        <button
                          class="process-link"
                          on:click={() => focusProcess(connection.pid)}
                          title={$t("ports.focusProcess")}
                        >
                          {processNameByPid.get(connection.pid) ?? "-"}
                        </button>
                      </td>
                      {@render trafficCells(connection)}
                      <td class="actions-col">
                        {@render rowActions(connection)}
                      </td>
                    </tr>
                    {@render detailRow(connection)}
                  {/each}
                {/if}
              {/each}
              {#if displayedGroups.length === 0}
                <tr>
                  <td class="empty-row" colspan={showSpeedCols ? 9 : 7}
                    >{$t("ports.noResults")}</td
                  >
                </tr>
              {/if}
            {:else}
              {#each flatTreeRows as row (row.kind === "node" ? row.node.pid : connectionKey(row.connection))}
                {#if row.kind === "node"}
                  <tr
                    class="group-row tree-node"
                    class:structural={row.node.ownCount === 0}
                    class:leaf={row.node.ownCount === 0 &&
                      row.node.children.length === 0}
                    style="--tree-indent: {row.depth * 18}px"
                  >
                    <td colspan={showSpeedCols ? 9 : 7}>
                      <div class="group-head-row">
                        <button
                          class="group-head"
                          on:click={() => toggleTreeNode(row.node.pid)}
                          aria-expanded={expandedTreeNodes.has(row.node.pid)}
                        >
                          <span class="group-caret">
                            <Fa icon={faCaretRight} />
                          </span>
                          <!-- svelte-ignore a11y_no_static_element_interactions -->
                          <span
                            class="group-name"
                            on:dblclick={() => openProcessDetails(row.node.pid)}
                            title={$t("ports.openDetails")}
                            >{row.node.name}</span
                          >
                          {#if row.node.pid}
                            <span class="group-pid"
                              >{$t("ports.pid")} {row.node.pid}</span
                            >
                          {/if}
                        </button>
                        {#if row.node.pid}
                          <div class="head-actions">
                            {#if row.node.ownCount > 0}
                              {@const favorited = row.node.connections.some(
                                (connection) =>
                                  favoriteKeys.has(portKeyOf(connection)),
                              )}
                              <button
                                class="focus-btn star-btn"
                                class:favorited
                                on:click={() =>
                                  toggleGroupFavorites(row.node.connections)}
                                title={favorited
                                  ? $t("ports.favoriteRemove")
                                  : $t("ports.favoriteAdd")}
                                aria-label={favorited
                                  ? $t("ports.favoriteRemove")
                                  : $t("ports.favoriteAdd")}
                              >
                                <Fa icon={faStar} />
                              </button>
                            {/if}
                            {#if canKillProcess(row.node.pid)}
                              <button
                                class="focus-btn danger"
                                on:click={() =>
                                  confirmKillProcess(
                                    row.node.pid,
                                    row.node.name,
                                  )}
                                title={$t("ports.killProcess")}
                                aria-label={$t("ports.killProcess")}
                              >
                                <Fa icon={faBan} />
                              </button>
                            {/if}
                          </div>
                        {/if}
                      </div>
                    </td>
                  </tr>
                  {#if expandedTreeNodes.has(row.node.pid)}
                    {#each row.node.connections as connection}
                      <tr
                        class="child-row"
                        style="--child-indent: {(row.depth + 1) * 18}px"
                        on:dblclick={(event) =>
                          handleRowDblClick(event, connection)}
                        in:fly={{ y: -4, duration: 150 }}
                        out:fade={{ duration: 100 }}
                      >
                        <td class="protocol">{connection.protocol}</td>
                        {@render localPortCell(connection)}
                        <td class="mono">
                          {#if connection.remote_addr}
                            {connection.remote_addr}:{connection.remote_port}
                          {:else}
                            -
                          {/if}
                        </td>
                        <td class="mono" title={connection.state}
                          >{localizedState(connection.state, $t)}</td
                        >
                        <td class="mono">{connection.pid || "-"}</td>
                        <td>
                          <button
                            class="process-link"
                            on:click={() => focusProcess(connection.pid)}
                            title={$t("ports.focusProcess")}
                          >
                            {processNameByPid.get(connection.pid) ?? "-"}
                          </button>
                        </td>
                        {@render trafficCells(connection)}
                        <td class="actions-col">
                          {@render rowActions(connection)}
                        </td>
                      </tr>
                      {@render detailRow(connection)}
                    {/each}
                  {/if}
                {/if}
              {/each}
              {#if flatTreeRows.length === 0}
                <tr>
                  <td class="empty-row" colspan={showSpeedCols ? 9 : 7}
                    >{$t("ports.noResults")}</td
                  >
                </tr>
              {/if}
            {/if}
          </tbody>
        </table>
      </div>

      <!-- Connection-manager-style totals: live speeds and cumulative bytes
           over everything that passed the current filters -->
      {#if showSpeedCols && filteredConnections.length > 0}
        {@const totals = sumTraffic(filteredConnections)}
        <div class="ports-summary mono">
          <span class="summary-item">↓ {speedText(totals.down)}</span>
          <span class="summary-item">↑ {speedText(totals.up)}</span>
          <span class="summary-item"
            >{$t("ports.received")} {formatBytes(totals.received)}</span
          >
          <span class="summary-item"
            >{$t("ports.sent")} {formatBytes(totals.sent)}</span
          >
        </div>
      {/if}
    {/if}
  </div>
</Modal>

<Modal
  show={connectionToClose !== null}
  title={$t("ports.closeTitle")}
  maxWidth="420px"
  onClose={cancelCloseConnection}
>
  {#if connectionToClose}
    <div class="confirm-content">
      <p class="confirm-message">{$t("ports.closeMessage")}</p>
      <div class="connection-info">
        <div class="connection-endpoints mono">
          {connectionToClose.local_addr}:{connectionToClose.local_port}
          →
          {connectionToClose.remote_addr}:{connectionToClose.remote_port}
        </div>
        <div class="connection-process">
          <span class="process-name">{connectionToCloseName}</span>
          <span class="connection-pid"
            >{$t("modal.pid", { pid: connectionToClose.pid })}</span
          >
        </div>
      </div>
      <p class="confirm-warning">{$t("ports.closeIrreversible")}</p>
      {#if closeError}
        <div class="ports-error">{closeError}</div>
      {/if}
      <div class="confirm-actions">
        <button
          class="btn-secondary"
          on:click={cancelCloseConnection}
          disabled={isClosingConnection}
        >
          {$t("modal.cancel")}
        </button>
        <button
          class="btn-danger"
          on:click={handleCloseConnection}
          disabled={isClosingConnection}
        >
          {#if isClosingConnection}
            <div class="spinner"></div>
            <span>{$t("ports.closeInProgress")}</span>
          {:else}
            {$t("ports.closeConfirm")}
          {/if}
        </button>
      </div>
    </div>
  {/if}
</Modal>

<Modal
  show={processToKill !== null}
  title={$t("ports.killProcess")}
  maxWidth="420px"
  onClose={cancelKillProcess}
>
  {#if processToKill}
    <div class="confirm-content">
      <p class="confirm-message">
        {$t("ports.killMessage", {
          name: processToKill.name,
          pid: processToKill.pid,
        })}
      </p>
      <div class="connection-info">
        <div class="connection-process">
          <span class="process-name">{processToKill.name}</span>
          <span class="connection-pid"
            >{$t("modal.pid", { pid: processToKill.pid })}</span
          >
        </div>
      </div>
      {#if killError}
        <div class="ports-error">{killError}</div>
      {/if}
      <div class="confirm-actions">
        <button
          class="btn-secondary"
          on:click={cancelKillProcess}
          disabled={isKillingProcess}
        >
          {$t("modal.cancel")}
        </button>
        <button
          class="btn-danger"
          on:click={handleKillProcess}
          disabled={isKillingProcess}
        >
          {#if isKillingProcess}
            <div class="spinner"></div>
            <span>{$t("kill.inProgress")}</span>
          {:else}
            {$t("kill.confirm")}
          {/if}
        </button>
      </div>
    </div>
  {/if}
</Modal>

<style>
  .ports-content {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .ports-toolbar {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .ports-search {
    flex: 1;
    height: 28px;
    padding: 0 10px;
    font-size: 13px;
    color: var(--text);
    background: var(--mantle);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    outline: none;
    box-sizing: border-box;
    transition: all 0.2s ease;
  }

  .ports-search:focus {
    border-color: var(--blue);
  }

  /* Icon-only view switch, aligned with the main toolbar's toggle */
  .view-toggle {
    display: inline-flex;
    align-items: center;
    height: 28px;
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    overflow: hidden;
  }

  .view-toggle button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 100%;
    padding: 0;
    font-size: 12px;
    color: var(--subtext0);
    background: transparent;
    border: none;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .view-toggle button:hover {
    color: var(--text);
    background: var(--surface1);
  }

  .view-toggle button.active {
    color: var(--base);
    background: var(--blue);
  }

  /* Icon-only collapse-all sitting beside the view switch, matching its
     button box; hidden in the flat list view where there is nothing to fold */
  .collapse-toggle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 28px;
    padding: 0;
    font-size: 12px;
    color: var(--subtext0);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .collapse-toggle:hover {
    color: var(--text);
    background: var(--surface1);
  }

  .ports-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }

  .chip-label {
    font-size: 12px;
    color: var(--subtext0);
  }

  .chip {
    height: 24px;
    padding: 0 10px;
    font-size: 12px;
    color: var(--subtext0);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 999px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .chip:hover {
    color: var(--text);
    background: var(--surface1);
  }

  .chip.active {
    color: var(--base);
    background: var(--blue);
    border-color: var(--blue);
  }

  .chip-divider {
    width: 1px;
    height: 16px;
    margin: 0 4px;
    background: var(--surface1);
  }

  .star-chip {
    display: inline-flex;
    gap: 6px;
    align-items: center;
  }

  .star-chip :global(svg) {
    font-size: 10px;
  }

  .focus-row {
    display: flex;
  }

  .focus-chip {
    display: inline-flex;
    gap: 6px;
    align-items: center;
    height: 26px;
    padding: 0 4px 0 10px;
    font-size: 12px;
    color: var(--text);
    background: var(--mantle);
    border: 1px solid var(--blue);
    border-radius: 999px;
  }

  .focus-chip :global(svg) {
    font-size: 11px;
    color: var(--blue);
  }

  .focus-pid {
    color: var(--subtext0);
  }

  .focus-clear {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    color: var(--subtext0);
    background: none;
    border: none;
    border-radius: 50%;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .focus-clear:hover {
    color: var(--red);
    background: var(--surface1);
  }

  .focus-clear :global(svg) {
    font-size: 11px;
  }

  .count-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    align-items: baseline;
    font-size: 12px;
    color: var(--subtext0);
  }

  .traffic-hint {
    margin-left: auto;
    color: var(--overlay0);
  }

  /* Watched-port chips: click removes the watch; yellow while a process
     is currently listening on the port */
  .watched-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 6px;
    align-items: center;
    font-size: 12px;
    color: var(--subtext0);
  }

  .watched-label {
    color: var(--overlay0);
  }

  .watch-chip {
    display: inline-flex;
    gap: 5px;
    align-items: center;
    padding: 1px 7px;
    font-family: inherit;
    font-size: 11px;
    color: var(--overlay1);
    cursor: pointer;
    background: var(--surface0);
    border: none;
    border-radius: 999px;
  }

  .watch-chip :global(svg) {
    width: 10px;
    height: 10px;
  }

  .watch-chip.live {
    color: var(--yellow);
    background: color-mix(in srgb, var(--yellow) 13%, transparent);
  }

  .watch-chip:hover {
    color: var(--red);
  }

  .watch-x {
    font-size: 12px;
    line-height: 1;
    opacity: 0.7;
  }

  .table-wrap {
    max-height: 60vh;
    overflow: auto;
    border: 1px solid var(--surface0);
    border-radius: 6px;
    scrollbar-width: thin;
    scrollbar-color: var(--surface2) var(--mantle);
  }

  /* Themed scrollbars: the WebView default (chunky, light, with arrow
     buttons) bleeds a jarring pale strip through the header's right end */
  .table-wrap::-webkit-scrollbar {
    width: 8px;
    height: 8px;
  }

  .table-wrap::-webkit-scrollbar-track {
    background: var(--mantle);
  }

  .table-wrap::-webkit-scrollbar-thumb {
    background: var(--surface2);
    border-radius: 4px;
  }

  .table-wrap::-webkit-scrollbar-thumb:hover {
    background: var(--surface1);
  }

  .table-wrap::-webkit-scrollbar-corner {
    background: var(--mantle);
  }

  table {
    width: 100%;
    /* "separate" keeps the sticky header painting reliably: with
       "collapse" the shared border layer belongs to the table and the
       header's background intermittently drops out over the trailing
       columns while scrolling */
    border-collapse: separate;
    border-spacing: 0;
    font-size: 12px;
  }

  thead th {
    position: sticky;
    top: 0;
    /* Paint above the positioned tbody cells scrolling underneath */
    z-index: 2;
    padding: 8px 10px;
    font-weight: 500;
    text-align: left;
    white-space: nowrap;
    color: var(--subtext0);
    background: var(--mantle);
    border-bottom: 1px solid var(--surface0);
  }

  .th-sort {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 0;
    font: inherit;
    font-weight: 500;
    color: inherit;
    text-align: left;
    background: none;
    border: none;
    cursor: pointer;
    user-select: none;
  }

  .sort-indicator {
    font-size: 12px;
    color: var(--overlay0);
    opacity: 0.5;
    transition: all 0.2s ease;
  }

  .th-sort:hover .sort-indicator {
    opacity: 1;
  }

  /* Tree view orders itself by aggregate connection count; the column
     sorters are disabled there */
  .th-sort:disabled {
    cursor: default;
  }

  .sort-indicator.active {
    color: var(--blue);
    opacity: 1;
  }

  /* Wide enough for the busiest cluster (star, details, browser, copy,
     focus, kill, close) */
  .actions-col {
    width: 198px;
    text-align: center;
  }

  /* Anchor for the transient row notice. This MUST stay on the td only:
     the same class sits on the header cell, and position:relative there
     would override thead th's position:sticky (class beats type
     selector), making the actions header scroll away with the body and
     punching a hole in the stuck header row */
  td.actions-col {
    position: relative;
  }

  tbody td {
    padding: 6px 10px;
    white-space: nowrap;
    color: var(--text);
    border-bottom: 1px solid var(--surface0);
  }

  tbody tr:last-child td {
    border-bottom: none;
  }

  tbody tr:hover {
    background: var(--mantle);
  }

  .process-link {
    padding: 0;
    font: inherit;
    color: inherit;
    text-align: left;
    background: none;
    border: none;
    cursor: pointer;
  }

  .process-link:hover {
    color: var(--blue);
    text-decoration: underline;
  }

  .focus-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    color: var(--subtext0);
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    opacity: 0;
    transition: all 0.15s ease;
  }

  tbody tr:hover .focus-btn,
  .focus-btn:focus-visible,
  .focus-btn.active {
    opacity: 1;
  }

  .focus-btn:hover {
    color: var(--blue);
    background: var(--surface1);
  }

  .focus-btn.active {
    color: var(--blue);
  }

  .focus-btn :global(svg) {
    font-size: 11px;
  }

  .row-actions {
    display: inline-flex;
    gap: 4px;
    align-items: center;
  }

  /* Failure hint of the port browser actions: a transient line under the
     offending row's action cluster instead of an alert dialog */
  .row-notice {
    position: absolute;
    top: 100%;
    right: 6px;
    z-index: 2;
    padding: 2px 8px;
    font-size: 11px;
    color: var(--red);
    background: var(--mantle);
    border-radius: 4px;
    white-space: nowrap;
  }

  .focus-btn.danger:hover {
    color: var(--red);
    border-color: var(--red);
  }

  /* Favorite star: dim until hovered, steady yellow when the port is
     favorited so pinned rows stay recognizable without hovering */
  .focus-btn.star-btn:hover {
    color: var(--yellow);
    background: var(--surface1);
  }

  .focus-btn.star-btn.favorited {
    color: var(--yellow);
    opacity: 1;
  }

  /* "Copy address" success feedback: the icon swaps to a check for a moment
     and stays visible even if the pointer has already left the row */
  .focus-btn.copied {
    color: var(--green);
    opacity: 1;
  }

  /* --- Port deep-dive panel ("Why is this running?") ---
     Inline expansion under the selected row; the ancestry chain is the
     visual protagonist, everything else stays quiet */
  .detail-row td {
    padding: 0;
    background: var(--mantle);
    white-space: normal;
  }

  .detail-panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border-left: 2px solid var(--blue);
  }

  .detail-warnings {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .warn-badge {
    display: inline-flex;
    gap: 5px;
    align-items: center;
    padding: 2px 8px;
    font-size: 11px;
    white-space: nowrap;
    border-radius: 4px;
  }

  .warn-badge :global(svg) {
    font-size: 10px;
  }

  .warn-badge.red {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--red) 45%, transparent);
  }

  .warn-badge.yellow {
    color: var(--yellow);
    background: color-mix(in srgb, var(--yellow) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--yellow) 45%, transparent);
  }

  .detail-chain-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .chain-label {
    font-size: 11px;
    color: var(--subtext0);
  }

  .detail-chain {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    font-size: 13px;
  }

  .chain-seg {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    max-width: 260px;
    padding: 3px 9px;
    font-weight: 500;
    color: var(--text);
    background: var(--surface1);
    border-radius: 6px;
  }

  .chain-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chain-pid {
    flex-shrink: 0;
    font-family: monospace;
    font-size: 11px;
    font-weight: 400;
    color: var(--subtext0);
  }

  .chain-seg.subject {
    color: var(--blue);
    background: color-mix(in srgb, var(--blue) 14%, transparent);
  }

  .chain-seg.subject .chain-pid {
    color: var(--blue);
    opacity: 0.75;
  }

  .chain-arrow {
    color: var(--overlay0);
  }

  .chain-broken {
    font-size: 12px;
    font-style: italic;
    color: var(--subtext0);
  }

  .detail-facts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 8px 16px;
  }

  .fact {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .fact-label {
    font-size: 11px;
    color: var(--subtext0);
  }

  .fact-value {
    overflow: hidden;
    font-size: 12px;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .fact-value.mono {
    font-family: monospace;
  }

  .fact-pid {
    font-size: 11px;
    color: var(--subtext0);
  }

  .detail-command {
    padding: 8px 10px;
    font-family: monospace;
    font-size: 11px;
    line-height: 1.5;
    color: var(--text);
    overflow-wrap: anywhere;
    white-space: pre-wrap;
    word-break: break-all;
    background: var(--surface0);
    border-radius: 6px;
  }

  .detail-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    justify-content: flex-end;
  }

  .detail-actions .btn-secondary,
  .detail-actions .btn-danger {
    padding: 5px 12px;
    font-size: 12px;
  }

  .group-row td {
    padding: 0;
    background: var(--surface0);
  }

  .group-head {
    display: flex;
    gap: 8px;
    align-items: center;
    width: 100%;
    padding: 6px 10px;
    font: inherit;
    text-align: left;
    background: none;
    border: none;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .group-head:hover {
    background: var(--mantle);
  }

  /* Head rows wrap the clickable head button with trailing quick actions
     (favorite star, kill); HTML forbids nested buttons, so the actions sit
     beside the head button instead of inside it */
  .group-head-row {
    display: flex;
    align-items: center;
    width: 100%;
  }

  .group-head-row .group-head {
    flex: 1;
    min-width: 0;
    width: auto;
  }

  .group-head-row:hover {
    background: var(--mantle);
  }

  .head-actions {
    display: inline-flex;
    flex-shrink: 0;
    gap: 4px;
    align-items: center;
    padding: 0 8px 0 4px;
  }

  .group-caret {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    width: 14px;
  }

  .group-caret :global(svg) {
    font-size: 11px;
    color: var(--subtext0);
    transition: transform 0.2s ease;
  }

  .group-head[aria-expanded="true"] .group-caret :global(svg) {
    transform: rotate(90deg);
  }

  .group-name {
    overflow: hidden;
    min-width: 0;
    font-weight: 600;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .group-pid {
    flex-shrink: 0;
    padding: 1px 6px;
    font-family: monospace;
    font-size: 10px;
    color: var(--subtext0);
    background: var(--surface1);
    border-radius: 4px;
  }

  tbody tr.child-row td:first-child {
    padding-left: var(--child-indent, 24px);
  }

  /* Tree view: node rows reuse the group-head language, indented 18px per
     depth level; structural nodes (no own connections, descendants only)
     are weakened so the eye keeps the connected processes as the subject */
  .tree-node .group-head {
    padding-left: calc(10px + var(--tree-indent, 0px));
  }

  .tree-node.structural .group-name {
    font-weight: 400;
    color: var(--subtext0);
  }

  /* Nodes with nothing to expand (possible after cycle re-rooting) keep the
     caret slot for alignment but render it invisible */
  .tree-node.leaf .group-caret {
    visibility: hidden;
  }

  .mono {
    font-family: monospace;
    font-size: 12px;
  }

  /* Traffic speed cells: quiet until something actually flows */
  .speed {
    color: var(--subtext0);
  }

  /* Bottom totals bar, Connection-Manager style */
  .ports-summary {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    align-items: center;
    padding: 6px 10px;
    font-size: 12px;
    color: var(--subtext0);
    background: var(--surface0);
    border-radius: 6px;
  }

  .summary-item {
    white-space: nowrap;
  }

  /* Port category tags (well-known local ports) */
  .port-cat {
    margin-left: 6px;
    padding: 1px 5px;
    font-family: inherit;
    font-size: 10px;
    border-radius: 4px;
    white-space: nowrap;
  }

  .port-cat.cat-web {
    color: var(--blue);
    background: color-mix(in srgb, var(--blue) 14%, transparent);
  }

  .port-cat.cat-database {
    color: var(--peach);
    background: color-mix(in srgb, var(--peach) 14%, transparent);
  }

  .port-cat.cat-dev {
    color: var(--teal);
    background: color-mix(in srgb, var(--teal) 14%, transparent);
  }

  .port-cat.cat-system {
    color: var(--maroon);
    background: color-mix(in srgb, var(--maroon) 14%, transparent);
  }

  .port-cat.cat-proxy {
    color: var(--sapphire);
    background: color-mix(in srgb, var(--sapphire) 14%, transparent);
  }

  .port-cat.cat-mail {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 14%, transparent);
  }

  /* Probed service role (SOCKS/mixed/controller/...): neutral chrome, the
     controller verdict gets the accent since it is the actionable one */
  .role-tag {
    margin-left: 6px;
    padding: 1px 5px;
    font-family: inherit;
    font-size: 10px;
    border-radius: 4px;
    white-space: nowrap;
    color: var(--text);
    background: color-mix(in srgb, var(--overlay0) 22%, transparent);
  }

  .role-tag.role-controller,
  .role-tag.role-web {
    color: var(--blue);
    background: color-mix(in srgb, var(--blue) 14%, transparent);
  }

  .role-tag.role-mixed,
  .role-tag.role-socks,
  .role-tag.role-http_proxy {
    color: var(--sapphire);
    background: color-mix(in srgb, var(--sapphire) 14%, transparent);
  }

  /* Bind scope hint (all interfaces vs local only) — quieter than tags */
  .bind-scope {
    margin-left: 6px;
    font-family: inherit;
    font-size: 10px;
    white-space: nowrap;
    color: var(--overlay0);
  }

  /* Integrity level badge in the detail panel */
  .integrity-badge {
    padding: 1px 6px;
    font-size: 11px;
    border-radius: 4px;
    background: var(--surface1);
  }

  .integrity-badge.integrity-high,
  .integrity-badge.integrity-system,
  .integrity-badge.integrity-protected {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 12%, transparent);
  }

  .integrity-badge.integrity-low,
  .integrity-badge.integrity-untrusted {
    color: var(--yellow);
    background: color-mix(in srgb, var(--yellow) 12%, transparent);
  }

  /* Authenticode verdict badge in the detail panel */
  .sig-badge {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    padding: 1px 6px;
    font-size: 11px;
    border-radius: 4px;
  }

  .sig-badge :global(svg) {
    width: 11px;
    height: 11px;
  }

  .sig-badge.signed {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 12%, transparent);
  }

  .sig-badge.unsigned {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 12%, transparent);
  }

  /* Watched-port bell in the row actions */
  .focus-btn.watch-btn.active {
    color: var(--yellow);
  }

  /* witr-style relations: children and siblings of the detail process */
  .detail-relations {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .relation-row {
    display: flex;
    gap: 10px;
    align-items: baseline;
  }

  .relation-label {
    flex-shrink: 0;
    font-size: 11px;
    color: var(--subtext0);
  }

  .relation-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    min-width: 0;
  }

  .relation-chip {
    max-width: 220px;
    padding: 1px 8px;
    font-size: 11px;
    color: var(--text);
    background: var(--surface0);
    border: none;
    border-radius: 999px;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    transition: all 0.15s ease;
  }

  .relation-chip:hover {
    color: var(--blue);
    background: var(--surface1);
  }

  .relation-more,
  .relation-none {
    font-size: 11px;
    color: var(--overlay0);
  }

  .protocol {
    font-weight: 600;
    color: var(--blue);
  }

  .empty-row {
    padding: 16px;
    text-align: center;
    color: var(--subtext0);
  }

  .load-more-row td {
    padding: 0;
  }

  .load-more {
    width: 100%;
    padding: 8px;
    font: inherit;
    color: var(--subtext0);
    background: none;
    border: none;
    cursor: pointer;
  }

  .load-more:hover {
    color: var(--text);
    background: var(--surface0);
  }

  .ports-status {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 32px;
    font-size: 13px;
    color: var(--subtext0);
  }

  .ports-error {
    padding: 8px 12px;
    font-size: 13px;
    color: var(--red);
    background: var(--surface0);
    border: 1px solid var(--red);
    border-radius: 6px;
  }

  /* Close-connection confirmation, styled after KillProcessModal */
  .confirm-content {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .confirm-message {
    margin: 0;
    font-size: 14px;
    color: var(--text);
  }

  .connection-info {
    padding: 12px;
    background: var(--mantle);
    border-radius: 6px;
  }

  .connection-endpoints {
    font-size: 12px;
    color: var(--text);
  }

  .connection-process {
    margin-top: 6px;
    font-size: 12px;
  }

  .process-name {
    font-weight: 500;
    color: var(--text);
  }

  .connection-pid {
    margin-left: 8px;
    font-size: 11px;
    color: var(--subtext0);
  }

  .confirm-warning {
    margin: 0;
    font-size: 12px;
    color: var(--yellow);
  }

  .confirm-actions {
    display: flex;
    gap: 12px;
    justify-content: flex-end;
  }

  .btn-secondary {
    padding: 8px 16px;
    font-size: 13px;
    color: var(--text);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .btn-secondary:hover {
    background: var(--surface1);
  }

  .btn-danger {
    display: inline-flex;
    gap: 8px;
    align-items: center;
    padding: 8px 16px;
    font-size: 13px;
    color: var(--base);
    background: var(--red);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .btn-danger:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .btn-danger:hover {
    background: color-mix(in srgb, var(--red) 90%, white);
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--surface1);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
