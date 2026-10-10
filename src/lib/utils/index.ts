import { get } from "svelte/store";
import type { Process, ProcessTreeRow, SortConfig } from "$lib/types";
import { t } from "$lib/i18n";
import { isElevated } from "$lib/stores/elevation";

export interface ProcessStatus {
  label: string;
  emoji: string;
  color: string;
}

export function formatMemorySize(bytes: number): string {
  const gb = bytes / (1024 * 1024 * 1024);
  return `${gb.toFixed(1)} GB`;
}

export function formatPercentage(value: number): string {
  return `${value.toFixed(1)}%`;
}

export function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return `${days}d ${hours}h ${minutes}m`;
}

export function getUsageClass(percentage: number): string {
  if (percentage >= 90) return "critical";
  if (percentage >= 60) return "high";
  if (percentage >= 30) return "medium";
  return "low";
}

export function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex++;
  }

  return `${value.toFixed(1)} ${units[unitIndex]}`;
}

export function formatDate(timestamp: number) {
  return new Date(timestamp * 1000).toLocaleString();
}

// Debounce utility to reduce frequency of expensive operations
export function debounce<T extends (...args: any[]) => any>(
  func: T,
  wait: number,
): (...args: Parameters<T>) => void {
  let timeout: ReturnType<typeof setTimeout>;
  return (...args: Parameters<T>) => {
    clearTimeout(timeout);
    timeout = setTimeout(() => func(...args), wait);
  };
}

// Cache for compiled regex patterns
const regexCache = new Map<string, RegExp>();

/**
 * Search relevance tier of a hit. Tier 0 = the process NAME itself
 * matches (what the user is usually after); tier 1 = only the command
 * line or the PID matches (contextual — "work" also lives inside every
 * *NetworkService* argument, so a bare field OR-match buries the real
 * name hits under browser helpers).
 */
export type SearchTier = 0 | 1;

/**
 * Splits the raw search box content into terms. Empty terms (a trailing
 * comma would produce one, and the empty string is a substring of
 * everything) are dropped instead of matching the whole list.
 */
export function searchTerms(searchTerm: string): string[] {
  return searchTerm
    .split(",")
    .map((term) => term.trim())
    .filter((term) => term.length > 0);
}

/**
 * Relevance tier of `process` against the search terms, or null when it
 * does not match at all. Name hits (substring, or regex for pattern-y
 * terms) are tier 0 and win immediately; command-line/PID hits are the
 * tier-1 fallback. Shared by filterProcesses and buildSearchTiers so the
 * filter and the rank can never disagree.
 */
export function searchMatchTier(
  process: Process,
  terms: string[],
): SearchTier | null {
  if (terms.length === 0) return null;
  const nameLower = process.name.toLowerCase();
  const commandLower = process.command.toLowerCase();
  const pidString = process.pid.toString();

  let contextual = false;
  for (const term of terms) {
    const termLower = term.toLowerCase();
    if (nameLower.includes(termLower)) return 0;
    if (
      !contextual &&
      (commandLower.includes(termLower) || pidString.includes(term))
    ) {
      contextual = true;
    }
  }
  if (contextual) return 1;

  // Regex pass on the name only (the command line never had one, same as
  // before); invalid patterns simply never match.
  for (const term of terms) {
    try {
      let regex = regexCache.get(term);
      if (!regex) {
        regex = new RegExp(term, "i");
        regexCache.set(term, regex);
      }
      if (regex.test(process.name)) return 0;
    } catch {
      // Invalid pattern: the substring pass already had its chance.
    }
  }
  return null;
}

/**
 * Tier per pid for the current filtered list, consumed by sortProcesses so
 * name hits lead the flat list while the user's sort column orders within
 * each tier. Empty when no search is active.
 */
export function buildSearchTiers(
  processes: Process[],
  searchTerm: string,
): Map<number, SearchTier> {
  const tiers = new Map<number, SearchTier>();
  const terms = searchTerms(searchTerm);
  if (terms.length === 0) return tiers;
  for (const process of processes) {
    const tier = searchMatchTier(process, terms);
    if (tier !== null) tiers.set(process.pid, tier);
  }
  return tiers;
}

export function filterProcesses(
  processes: Process[],
  searchTerm: string,
  filters: {
    cpu: { operator: string; value: number; enabled: boolean };
    ram: { operator: string; value: number; enabled: boolean };
    runtime: { operator: string; value: number; enabled: boolean };
    status: { values: string[]; enabled: boolean };
  },
): Process[] {
  // Early return for empty search and no active filters
  if (
    searchTerm.length === 0 &&
    !Object.values(filters).some((f) => f.enabled)
  ) {
    return processes;
  }

  // Pre-process search terms once
  const terms = searchTerm.length > 0 ? searchTerms(searchTerm) : [];

  return processes.filter((process) => {
    // Apply status filter
    if (filters.status.enabled && filters.status.values.length > 0) {
      if (!filters.status.values.includes(process.status)) {
        return false;
      }
    }

    // Apply CPU filter
    if (filters.cpu.enabled) {
      const cpuValue = process.cpu_usage;
      if (!compareValue(cpuValue, filters.cpu.operator, filters.cpu.value)) {
        return false;
      }
    }

    // Apply RAM filter (convert bytes to MB)
    if (filters.ram.enabled) {
      const ramMB = process.memory_usage / (1024 * 1024);
      if (!compareValue(ramMB, filters.ram.operator, filters.ram.value)) {
        return false;
      }
    }

    // Apply runtime filter (convert to minutes)
    if (filters.runtime.enabled) {
      const runtimeMin = process.run_time / 60;
      if (
        !compareValue(
          runtimeMin,
          filters.runtime.operator,
          filters.runtime.value,
        )
      ) {
        return false;
      }
    }

    // Skip search if no terms
    if (terms.length === 0) {
      return true;
    }

    // The filter only needs the verdict, not the tier
    return searchMatchTier(process, terms) !== null;
  });
}

// Helper function to compare values based on operator
function compareValue(
  value: number,
  operator: string,
  target: number,
): boolean {
  switch (operator) {
    case ">":
      return value > target;
    case "<":
      return value < target;
    case "=":
      return value === target;
    case ">=":
      return value >= target;
    case "<=":
      return value <= target;
    default:
      return true;
  }
}

// Compare two processes by the active sort field; shared by the flat list
// sort and the tree view (where it orders siblings within each level).
export function compareProcesses(
  a: Process,
  b: Process,
  sortConfig: SortConfig,
): number {
  const direction = sortConfig.direction === "asc" ? 1 : -1;
  const aValue = a[sortConfig.field];
  const bValue = b[sortConfig.field];

  // Special handling for disk_usage which is an array [read_bytes, written_bytes]
  if (sortConfig.field === "disk_usage") {
    const aRead = (aValue as [number, number])[0];
    const aWrite = (aValue as [number, number])[1];
    const bRead = (bValue as [number, number])[0];
    const bWrite = (bValue as [number, number])[1];

    // Smart sorting: analyze if this is a read-heavy or write-heavy comparison
    const totalReads = aRead + bRead;
    const totalWrites = aWrite + bWrite;

    if (totalWrites > totalReads * 1.5) {
      // Write-heavy scenario: prioritize writes, use reads as tiebreaker
      if (aWrite !== bWrite) {
        return direction * (aWrite - bWrite);
      }
      return direction * (aRead - bRead);
    } else if (totalReads > totalWrites * 1.5) {
      // Read-heavy scenario: prioritize reads, use writes as tiebreaker
      if (aRead !== bRead) {
        return direction * (aRead - bRead);
      }
      return direction * (aWrite - bWrite);
    } else {
      // Balanced I/O: sort by total, use max as tiebreaker
      const aTotalDisk = aRead + aWrite;
      const bTotalDisk = bRead + bWrite;
      if (aTotalDisk !== bTotalDisk) {
        return direction * (aTotalDisk - bTotalDisk);
      }
      // Tiebreaker: use the dominant operation
      const aMaxDisk = Math.max(aRead, aWrite);
      const bMaxDisk = Math.max(bRead, bWrite);
      return direction * (aMaxDisk - bMaxDisk);
    }
  }

  // Type-specific comparisons
  if (typeof aValue === "string") {
    return direction * aValue.localeCompare(bValue as string);
  }
  return direction * (Number(aValue) - Number(bValue));
}

/**
 * Pin priority: "float to top". Pinned processes lead the list in pin
 * order (the order they were pinned, stable across refreshes) and
 * everything else sorts by the user's sort field. This is the core
 * 置顶 semantic — a pinned row must sit above all unpinned rows no
 * matter which column is sorted, otherwise pinning does nothing
 * observable on look-alike lists.
 */
function pinOrderMap(pinnedPids: Iterable<number>): Map<number, number> {
  const order = new Map<number, number>();
  let index = 0;
  for (const pid of pinnedPids) order.set(pid, index++);
  return order;
}

/**
 * Search-tier comparison: with `searchTiers` (non-empty while a search is
 * active) name hits rank above command-line/PID-only hits, each tier still
 * ordered by the sort field — so searching "work" surfaces WorkBuddyAI.exe
 * instead of scattering it among every *NetworkService* helper. Shared by
 * the flat sort and both grouped views.
 */
function compareWithSearchTiers(
  a: Process,
  b: Process,
  sortConfig: SortConfig,
  searchTiers?: Map<number, SearchTier>,
): number {
  if (searchTiers && searchTiers.size > 0) {
    const tierDiff =
      (searchTiers.get(a.pid) ?? 0) - (searchTiers.get(b.pid) ?? 0);
    if (tierDiff !== 0) return tierDiff;
  }
  return compareProcesses(a, b, sortConfig);
}

/**
 * Pin-first comparison used by both the flat sort and the tree roots.
 * Pinned processes stay above both search tiers.
 */
function compareWithPinsFirst(
  a: Process,
  b: Process,
  pinOrder: Map<number, number>,
  sortConfig: SortConfig,
  searchTiers?: Map<number, SearchTier>,
): number {
  const aPin = pinOrder.get(a.pid);
  const bPin = pinOrder.get(b.pid);
  if (aPin !== undefined || bPin !== undefined) {
    if (aPin === undefined) return 1;
    if (bPin === undefined) return -1;
    return aPin - bPin;
  }
  return compareWithSearchTiers(a, b, sortConfig, searchTiers);
}

export function sortProcesses(
  processes: Process[],
  sortConfig: SortConfig,
  pinnedPids: Iterable<number> = new Set<number>(),
  searchTiers?: Map<number, SearchTier>,
): Process[] {
  const pinOrder = pinOrderMap(pinnedPids);
  return [...processes].sort((a, b) =>
    compareWithPinsFirst(a, b, pinOrder, sortConfig, searchTiers),
  );
}

/**
 * Bounds of the auto-fitted process-name column width.
 */
const NAME_WIDTH_MIN = 160;
const NAME_WIDTH_MAX = 420;
/** Extra room beyond the widest name: cell padding (12px each side),
 * the row icon (16px) and the name-cell flex gap (8px). */
const NAME_CELL_CHROME = 24 + 16 + 8;
/** Tree view adds an expand-arrow slot (16px + 8px gap) and indents
 * each level by 18px (see ProcessRow). */
const NAME_TREE_SLOT = 16 + 8;
const NAME_TREE_INDENT = 18;

/**
 * Measures the longest visible process name and returns the width the
 * name column needs to show it without an ellipsis, clamped to sane
 * bounds. Text is measured with canvas using the table's real font;
 * `treeMode` adds the expand-arrow slot and per-level indentation.
 */
export function computeNameColumnWidth(
  rows: ProcessTreeRow[],
  reference: HTMLElement,
  treeMode: boolean,
): number {
  if (typeof document === "undefined" || rows.length === 0) {
    return NAME_WIDTH_MIN;
  }
  const context = document.createElement("canvas").getContext("2d");
  if (!context) return NAME_WIDTH_MIN;
  // Rows inherit the table's font size and the body's font family.
  context.font = `${getComputedStyle(reference).fontSize} ${
    getComputedStyle(document.body).fontFamily
  }`;

  let widest = 0;
  for (const row of rows) {
    const textWidth = context.measureText(row.process.name).width;
    const extra =
      NAME_CELL_CHROME +
      (treeMode ? row.depth * NAME_TREE_INDENT + NAME_TREE_SLOT : 0);
    widest = Math.max(widest, textWidth + extra);
  }
  // Small buffer so the measured winner never lands on the ellipsis edge.
  return Math.min(
    NAME_WIDTH_MAX,
    Math.max(NAME_WIDTH_MIN, Math.ceil(widest) + 4),
  );
}

/**
 * Reports whether a ppid edge parent→child is stale. The kernel never
 * rewrites PEPID after the original parent exits, so a recycled PID (or a
 * cross-session "parent" that adopted the child) points at an unrelated
 * process. witr's ancestry guards: a real parent started no later than its
 * child, and a forked process inherits its session. Edges with an unknown
 * start time (0) or session are kept — they cannot be judged.
 */
function isStaleEdge(parent: Process, child: Process): boolean {
  if (
    parent.start_time > 0 &&
    child.start_time > 0 &&
    parent.start_time > child.start_time
  ) {
    return true;
  }
  return (
    parent.session_id !== undefined &&
    child.session_id !== undefined &&
    parent.session_id !== child.session_id
  );
}

/**
 * Expands a filtered process list with the ancestor chain of every hit so
 * the tree view keeps parent rows attached above matching children.
 * Cycles (self-referencing ppids) and missing parents are tolerated.
 */
export function withAncestors(matched: Process[], all: Process[]): Process[] {
  if (matched.length === all.length) return matched;
  const byPid = new Map(all.map((p) => [p.pid, p]));
  const visible = new Map<number, Process>();
  for (const process of matched) {
    visible.set(process.pid, process);
    let current = process;
    const visited = new Set<number>([process.pid]);
    for (;;) {
      const parent = byPid.get(current.ppid);
      // A stale edge (recycled PID / cross-session adoption) means the
      // chain above it is not the real ancestry — stop at the child
      if (!parent || visited.has(parent.pid) || isStaleEdge(parent, current))
        break;
      visited.add(parent.pid);
      visible.set(parent.pid, parent);
      current = parent;
    }
  }
  return [...visible.values()];
}

/**
 * Builds flattened tree rows from the visible process list, ordered by
 * ppid. Processes whose parent is not in the list (or is themselves, e.g.
 * PID reuse) are attached to the root. Pinned processes are hoisted out of
 * their parent to the top of the root level in pin order, their subtree
 * staying attached, so 置顶 reads as "always first" in tree view too.
 * Sorting otherwise applies to siblings within each level; a collapsed
 * path hides its whole subtree.
 *
 * Collapse state is keyed by the root-to-node name chain instead of the
 * PID: short-lived processes (browser helpers etc.) constantly change
 * PIDs, and a PID key makes collapsed groups re-expand on every churn.
 */
export function buildTreeRows(
  visible: Process[],
  sortConfig: SortConfig,
  collapsedPaths: Set<string>,
  pinnedPids: Iterable<number> = new Set<number>(),
  searchTiers?: Map<number, SearchTier>,
): ProcessTreeRow[] {
  const pinOrder = pinOrderMap(pinnedPids);
  const byPid = new Map(visible.map((p) => [p.pid, p]));
  const childrenOf = new Map<number | null, Process[]>();
  for (const process of visible) {
    const parent = byPid.get(process.ppid);
    // Pinned processes are re-rooted so they always lead the list; a stale
    // ppid edge (recycled or cross-session parent) is rooted the same way
    const hoisted = pinOrder.has(process.pid);
    const key =
      !hoisted &&
      parent &&
      parent.pid !== process.pid &&
      !isStaleEdge(parent, process)
        ? process.ppid
        : null;
    const siblings = childrenOf.get(key);
    if (siblings) {
      siblings.push(process);
    } else {
      childrenOf.set(key, [process]);
    }
  }

  const sortSiblings = (siblings: Process[], isRoot: boolean) => {
    if (isRoot && pinOrder.size > 0) {
      siblings.sort((a, b) =>
        compareWithPinsFirst(a, b, pinOrder, sortConfig, searchTiers),
      );
      return;
    }
    siblings.sort((a, b) =>
      compareWithSearchTiers(a, b, sortConfig, searchTiers),
    );
  };
  for (const [key, siblings] of childrenOf.entries()) {
    sortSiblings(siblings, key === null);
  }

  const rows: ProcessTreeRow[] = [];
  const walked = new Set<number>();
  // Walks the whole tree so every reachable process is marked, but only
  // pushes rows for rendered levels; behind a collapsed node the subtree
  // is still visited (marked) yet not rendered.
  const walk = (
    parentPid: number | null,
    parentPath: string,
    depth: number,
    render: boolean,
  ) => {
    for (const process of childrenOf.get(parentPid) ?? []) {
      walked.add(process.pid);
      const path =
        parentPath === "" ? process.name : `${parentPath}\u0001${process.name}`;
      const hasChildren = childrenOf.has(process.pid);
      const expanded = render && hasChildren && !collapsedPaths.has(path);
      if (render) {
        rows.push({ process, path, depth, hasChildren, expanded });
      }
      walk(process.pid, path, expanded ? depth + 1 : depth, expanded);
    }
  };
  walk(null, "", 0, true);
  // A ppid cycle (A's parent is B's, B's parent is A's, e.g. after PID
  // reuse) leaves both members unwalked; surface them at the root instead
  // of silently dropping the rows.
  for (const process of visible) {
    if (!walked.has(process.pid)) {
      rows.push({
        process,
        path: process.name,
        depth: 0,
        hasChildren: false,
        expanded: false,
      });
    }
  }
  return rows;
}

/**
 * Chromium-style data-directory identity: a process launched with
 * `--user-data-dir=<dir>` (WebView2 runtimes ALWAYS carry it; browsers
 * with a custom profile do too) belongs to the application that owns
 * that data directory, NOT to everyone running the same runtime binary —
 * without it every host's msedgewebview2 processes collapse into one
 * group and one kill. The value is matched up to the next " --" argument
 * (a limitation of the joined command string), trimmed of quotes, and
 * lowercased for keying; empty when absent. The backend's
 * `exe_identity_of` applies the same rule to its parsed argv, so the
 * confirmation-dialog count always equals the kill set.
 */
function commandDataDir(command: string): string {
  const marker = "--user-data-dir=";
  const idx = command.indexOf(marker);
  if (idx === -1) return "";
  let value = command.slice(idx + marker.length);
  const nextArg = value.indexOf(" --");
  if (nextArg !== -1) value = value.slice(0, nextArg);
  value = value.trim().replace(/^"|"$/g, "");
  return value;
}

/** Full application identity of a process: the executable path (the name
 * when the exe read was denied) plus, when present, the Chromium data
 * directory — two hosts embedding the same WebView2 runtime are two
 * applications. Mirrored by the backend's kill-app-family key. */
export function identityKeyOf(process: Process): string {
  const base = (process.exe.trim() ? process.exe : process.name).toLowerCase();
  const dataDir = commandDataDir(process.command);
  return dataDir ? `${base}|${dataDir.toLowerCase()}` : base;
}

/** Human-readable tail disambiguating same-runtime group names. Prefers
 * the WebView2 host name (`--webview-exe-name=host.exe` — the clearest
 * label); falls back to the data directory's identifying segment (the one
 * BEFORE the fixed "EBWebView" leaf on WebView2, the last segment for
 * anything else); empty when the process carries no --user-data-dir. */
function dataDirTail(command: string): string {
  const hostMarker = "--webview-exe-name=";
  const hostIdx = command.indexOf(hostMarker);
  if (hostIdx !== -1) {
    let host = command.slice(hostIdx + hostMarker.length);
    const nextArg = host.indexOf(" --");
    if (nextArg !== -1) host = host.slice(0, nextArg);
    host = host
      .trim()
      .replace(/^"|"$/g, "")
      .replace(/\.(exe|app)$/i, "");
    if (host) return host;
  }
  const dataDir = commandDataDir(command);
  if (!dataDir) return "";
  const segments = dataDir.split(/[\\/]/).filter(Boolean);
  if (segments.length === 0) return "";
  const last = segments[segments.length - 1];
  if (segments.length >= 2 && /^ebwebview$/i.test(last)) {
    return segments[segments.length - 2];
  }
  return last;
}

/**
 * Application scope of `pid`, the "End Application" semantics: EVERY
 * process of the target's session that shares its identity key — the
 * executable path plus the Chromium data directory when present —
 * regardless of parentage. A process running a *different* executable,
 * or one whose data directory (i.e. host application) differs, or sitting
 * in another session, is never part of the application: ending one
 * host's embedded WebView2 can no longer kill every other host's.
 */
export function appFamilyOf(processes: Process[], pid: number): Process[] {
  const target = processes.find((p) => p.pid === pid);
  if (!target) return [];
  const targetKey = identityKeyOf(target);
  // "End Application" scans the whole snapshot: every process of the
  // target's session sharing its identity, regardless of parentage — a
  // launcher-detached instance is still the same application. Unknown
  // session sides are kept (they cannot be judged). The backend
  // kill-app-family applies this exact same rule.
  return processes.filter(
    (p) =>
      identityKeyOf(p) === targetKey &&
      (target.session_id === undefined ||
        p.session_id === undefined ||
        p.session_id === target.session_id),
  );
}

/**
 * Sizes the application scope of `pid` from the snapshot (the estimate
 * the kill-the-app confirmation shows: every same-executable process of
 * the target's session); null when the target is missing.
 */
export function countAppFamilySize(
  processes: Process[],
  pid: number,
): number | null {
  const family = appFamilyOf(processes, pid);
  return family.length > 0 ? family.length : null;
}

/**
 * Builds the app-group rows of the tree view's "app" grouping: every
 * application (same executable within a session, Task Manager's "App (N)"
 * rows) renders as a leader row — the oldest-starting member carrying the
 * application's summed CPU/memory and a "(N)" name suffix — with the
 * remaining members nested one level below. Single-member applications
 * render as plain root rows. The leader row keeps the leader's real PID,
 * so row actions (details, kill, kill-app) target the supervising process.
 *
 * Collapse state is keyed by a stable group identity (`app:session|exe`),
 * NOT the display name — the display name carries the live member count,
 * so any member churn would re-expand collapsed groups on every refresh.
 * Application identity = the `exe` path (the `root` field is the cwd's
 * drive root on Windows — "/" on Linux — and would collapse every
 * same-drive process onto one key); when the exe read was denied the
 * name stands in, matching the backend's kill-app-family fallback.
 */
export function buildAppRows(
  visible: Process[],
  sortConfig: SortConfig,
  collapsedPaths: Set<string>,
  searchTiers?: Map<number, SearchTier>,
): ProcessTreeRow[] {
  const byKey = new Map<string, Process[]>();
  for (const process of visible) {
    // Application identity = executable + Chromium data directory +
    // session. The launcher that started the app (explorer.exe, a service
    // host, ...) is the app's PARENT, never a member, so no launcher's
    // co-applications bleed into its row; shared runtimes (msedgewebview2,
    // svchost, dotnet, ...) keep one row per owning application/session
    // thanks to the --user-data-dir segment of the key.
    const key = `app:${process.session_id ?? 0}|${identityKeyOf(process)}`;
    const members = byKey.get(key);
    if (members) {
      members.push(process);
    } else {
      byKey.set(key, [process]);
    }
  }

  // start_time 0 means "unknown"; those members sort last in leader picks
  const knownTime = (p: Process) =>
    p.start_time > 0 ? p.start_time : Number.MAX_SAFE_INTEGER;

  interface AppGroup {
    /** Stable identity ("app:session|exe") — the collapse-state key. */
    key: string;
    leader: Process;
    members: Process[]; // everyone except the leader
    /** Display name; carries the live "(N)" count, never used as a key. */
    path: string;
  }
  const groups: AppGroup[] = [];
  for (const [key, members] of byKey) {
    const leader = members.reduce((a, b) =>
      knownTime(b) < knownTime(a) ? b : a,
    );
    const rest = members.filter((m) => m.pid !== leader.pid);
    // Same-runtime groups (msedgewebview2 & friends) carry their host's
    // data-directory tail so sibling groups are tellable apart — including
    // single-member groups (a host running just its browser process).
    const hostTail = dataDirTail(leader.command);
    const hostSuffix = hostTail ? ` [${hostTail}]` : "";
    const path =
      rest.length > 0
        ? `${leader.name}${hostSuffix} (${rest.length + 1})`
        : `${leader.name}${hostSuffix}`;
    groups.push({ key, leader, members: rest, path });
  }

  // Leader rows (and the plain singletons) share the root level, sorted
  // by the active column using the leader's aggregated values; search
  // tiers rank the leader rows too (tier is read off the leader's pid)
  groups.sort((a, b) =>
    compareWithSearchTiers(leaderAgg(a), leaderAgg(b), sortConfig, searchTiers),
  );

  const rows: ProcessTreeRow[] = [];
  for (const group of groups) {
    if (group.members.length === 0) {
      // Single-member group: the host tail still disambiguates it from
      // sibling same-runtime groups (a host running just its browser
      // process); the leader's real fields stay intact for row actions.
      const hostTail = dataDirTail(group.leader.command);
      rows.push({
        process: hostTail
          ? { ...group.leader, name: `${group.leader.name} [${hostTail}]` }
          : group.leader,
        path: group.key,
        depth: 0,
        hasChildren: false,
        expanded: false,
      });
      continue;
    }
    const row = {
      ...group.leader,
      name: group.path,
      cpu_usage:
        group.members.reduce((s, m) => s + m.cpu_usage, 0) +
        group.leader.cpu_usage,
      memory_usage:
        group.members.reduce((s, m) => s + m.memory_usage, 0) +
        group.leader.memory_usage,
    };
    const expanded = !collapsedPaths.has(group.key);
    rows.push({
      process: row,
      path: group.key,
      depth: 0,
      hasChildren: true,
      expanded,
    });
    if (expanded) {
      const sortedMembers = [...group.members].sort((a, b) =>
        compareWithSearchTiers(a, b, sortConfig, searchTiers),
      );
      for (const member of sortedMembers) {
        rows.push({
          process: member,
          path: `${group.key}\u0001${member.name}`,
          depth: 1,
          hasChildren: false,
          expanded: false,
        });
      }
    }
  }
  return rows;

  function leaderAgg(group: AppGroup): Process {
    return {
      ...group.leader,
      name: group.path,
      cpu_usage:
        group.members.reduce((s, m) => s + m.cpu_usage, 0) +
        group.leader.cpu_usage,
      memory_usage:
        group.members.reduce((s, m) => s + m.memory_usage, 0) +
        group.leader.memory_usage,
    };
  }
}

// Backend error fragments that indicate the operation failed for lack of
// elevation. They come from process_control.rs (open_process/signal_pid),
// from sysinfo's kill failing on protected processes and from std::io
// "os error 5" (ERROR_ACCESS_DENIED) messages.
const elevationErrorPattern =
  /access denied|administrator|elevated|privileges|permission denied|failed to kill|windows error 5\b|os error 5\b|\beperm\b/i;

/**
 * Appends a recovery hint to backend error messages that look like
 * permission failures. When the app is not elevated the hint points at
 * the relaunch-as-admin flow; when it already IS elevated the same error
 * means the target is protected or has a higher integrity level — telling
 * the user to "relaunch as admin" would be a lie (the shield is already
 * green), so the hint explains the protected-process case instead.
 */
export function withElevationHint(message: string): string {
  if (!elevationErrorPattern.test(message)) {
    return message;
  }
  if (get(isElevated)) {
    return `${message} — ${get(t)("settings.elevationHintElevated")}`;
  }
  return `${message} — ${get(t)("settings.elevationHint")}`;
}
export {
  buildAncestryChain,
  type AncestryChain,
  type ChainSegment,
} from "./ancestry";
