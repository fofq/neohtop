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
  const terms =
    searchTerm.length > 0
      ? searchTerm.split(",").map((term) => term.trim())
      : [];

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

    // Cache lowercase values
    const processNameLower = process.name.toLowerCase();
    const processCommandLower = process.command.toLowerCase();
    const processPidString = process.pid.toString();

    // Check each term
    return terms.some((term) => {
      const termLower = term.toLowerCase();

      // Try exact matches first (faster)
      if (
        processNameLower.includes(termLower) ||
        processCommandLower.includes(termLower) ||
        processPidString.includes(term)
      ) {
        return true;
      }

      // Try regex match last (slower)
      try {
        let regex = regexCache.get(term);
        if (!regex) {
          regex = new RegExp(term, "i");
          regexCache.set(term, regex);
        }
        return regex.test(process.name);
      } catch {
        return false;
      }
    });
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

/** Pin-first comparison used by both the flat sort and the tree roots. */
function compareWithPinsFirst(
  a: Process,
  b: Process,
  pinOrder: Map<number, number>,
  sortConfig: SortConfig,
): number {
  const aPin = pinOrder.get(a.pid);
  const bPin = pinOrder.get(b.pid);
  if (aPin !== undefined || bPin !== undefined) {
    if (aPin === undefined) return 1;
    if (bPin === undefined) return -1;
    return aPin - bPin;
  }
  return compareProcesses(a, b, sortConfig);
}

export function sortProcesses(
  processes: Process[],
  sortConfig: SortConfig,
  pinnedPids: Iterable<number> = new Set<number>(),
): Process[] {
  const pinOrder = pinOrderMap(pinnedPids);
  if (pinOrder.size === 0) {
    return [...processes].sort((a, b) => compareProcesses(a, b, sortConfig));
  }
  return [...processes].sort((a, b) =>
    compareWithPinsFirst(a, b, pinOrder, sortConfig),
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
      if (!parent || visited.has(parent.pid)) break;
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
): ProcessTreeRow[] {
  const pinOrder = pinOrderMap(pinnedPids);
  const byPid = new Map(visible.map((p) => [p.pid, p]));
  const childrenOf = new Map<number | null, Process[]>();
  for (const process of visible) {
    const parent = byPid.get(process.ppid);
    // Pinned processes are re-rooted so they always lead the list
    const hoisted = pinOrder.has(process.pid);
    const key =
      !hoisted && parent && parent.pid !== process.pid ? process.ppid : null;
    const siblings = childrenOf.get(key);
    if (siblings) {
      siblings.push(process);
    } else {
      childrenOf.set(key, [process]);
    }
  }

  const sortSiblings = (siblings: Process[], isRoot: boolean) => {
    if (!isRoot || pinOrder.size === 0) {
      siblings.sort((a, b) => compareProcesses(a, b, sortConfig));
      return;
    }
    siblings.sort((a, b) => compareWithPinsFirst(a, b, pinOrder, sortConfig));
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
 * Estimates the size of the process tree rooted at `pid` from a snapshot:
 * the process itself plus every descendant reachable through ppid chains.
 * Returns null when the root is absent from the snapshot so the caller can
 * present an open-ended count ("1+") instead of a wrong one. Cycles
 * (self-referencing ppids, e.g. after PID reuse) are tolerated.
 */
export function countProcessTreeSize(
  processes: Process[],
  pid: number,
): number | null {
  if (!processes.some((process) => process.pid === pid)) {
    return null;
  }
  const childrenOf = new Map<number, number[]>();
  for (const process of processes) {
    // ppid 0 means "no parent recorded" and must never anchor a subtree
    if (process.ppid <= 0) continue;
    const siblings = childrenOf.get(process.ppid);
    if (siblings) {
      siblings.push(process.pid);
    } else {
      childrenOf.set(process.ppid, [process.pid]);
    }
  }
  let count = 0;
  const visited = new Set<number>([pid]);
  const queue = [pid];
  while (queue.length > 0) {
    const current = queue.pop()!;
    count++;
    for (const child of childrenOf.get(current) ?? []) {
      if (!visited.has(child)) {
        visited.add(child);
        queue.push(child);
      }
    }
  }
  return count;
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
