<script lang="ts">
  import { backToTop } from "$lib/actions/backToTop";
  import type { Process, ProcessTreeRow, Column } from "$lib/types";
  import { TableHeader, ProcessRow, ProcessContextMenu } from "$lib/components";
  import { computeNameColumnWidth } from "$lib/utils";

  export let processes: Process[];
  export let columns: Column[];
  export let systemStats: { memory_total: number } | null;
  export let sortConfig: { field: keyof Process; direction: "asc" | "desc" };
  export let pinnedProcesses: Set<number>;
  /** PIDs to flash green as newly started. */
  export let justStarted: Set<number>;
  export let highlightDurationMs: number;
  /** PIDs the app has suspended (amber row marker + resume button). */
  export let suspendedPids: Set<number> = new Set();
  /** Tree view rows; null renders the flat (paginated) list. */
  export let treeRows: ProcessTreeRow[] | null = null;
  /** Manually resized widths (px) persisted in AppConfig. */
  export let columnWidths: Record<string, number> = {};
  /** Persists the manual width map after a resize or reset. */
  export let onColumnWidthsCommit: (
    widths: Record<string, number>,
  ) => void = () => {};

  export let onToggleSort: (field: keyof Process) => void;
  export let onTogglePin: (pid: number) => void;
  export let onShowDetails: (process: Process) => void;
  export let onRestartProcess: (process: Process) => void;
  export let onToggleSuspend: (process: Process) => void;
  export let onKillProcess: (process: Process) => void;
  /** Opens the kill-application-family confirmation for the given process. */
  export let onKillAppProcess: (process: Process) => void = () => {};
  /** Toggles a subtree's collapse state, keyed by the row's name path. */
  export let onToggleExpand: (path: string) => void = () => {};
  /** Row hover events for the rich tooltip (see ProcessHoverCard). */
  export let onHoverTooltip: (
    process: Process,
    event: MouseEvent,
  ) => void = () => {};
  export let onHideTooltip: () => void = () => {};

  const ACTIONS_WIDTH = 190;

  // Tree view tints each root family with one accent so parents and their
  // descendants read as a group at a glance. Accents are mixed into the
  // theme's text color (40%), so the tint stays readable on every theme
  // and never fights the semantic red/green/yellow row states. Flat mode
  // stays untinted.
  const FAMILY_ACCENTS = [
    "--blue",
    "--lavender",
    "--sapphire",
    "--sky",
    "--teal",
    "--peach",
    "--maroon",
  ];

  function familyColorOf(path: string): string {
    let hash = 0;
    for (let i = 0; i < path.length; i++) {
      hash = (hash * 31 + path.charCodeAt(i)) >>> 0;
    }
    const accent = FAMILY_ACCENTS[hash % FAMILY_ACCENTS.length];
    return `color-mix(in srgb, var(${accent}) 40%, var(--text))`;
  }

  function familyColorFor(row: ProcessTreeRow): string {
    if (!treeRows) return "";
    // The first path segment is the family's root: every descendant of
    // that root hashes to the same color
    return familyColorOf(row.path.split("\u0001")[0]);
  }

  let tableElement: HTMLTableElement;
  /** Auto-fitted name column width (longest visible row name). */
  let nameAutoWidth: number | null = null;
  /** View mode the auto-fit ran for, so switching views re-measures. */
  let fittedTreeMode: boolean | null = null;

  // Both modes render the same row shape; flat mode is depth 0 without
  // expand arrows, and its path is just the process name.
  $: rows =
    treeRows ??
    processes.map((process) => ({
      process,
      path: process.name,
      depth: 0,
      hasChildren: false,
      expanded: false,
    }));

  $: visibleColumns = columns.filter((col) => col.visible);

  // Manual widths are re-seeded from the persisted config on every config
  // change so resets (columnWidths = {}) take effect immediately.
  $: widths = { ...columnWidths };
  // The auto-fitted name width applies unless a manual width overrides it.
  $: effectiveWidths =
    nameAutoWidth === null ? widths : { name: nameAutoWidth, ...widths };

  // Auto-fit the name column once data is on screen, and again when the
  // view mode flips (tree indentation widens the name cell content).
  $: if (
    tableElement &&
    rows.length > 0 &&
    (nameAutoWidth === null || fittedTreeMode !== (treeRows !== null))
  ) {
    fittedTreeMode = treeRows !== null;
    nameAutoWidth = computeNameColumnWidth(
      rows,
      tableElement,
      treeRows !== null,
    );
  }

  function handleColumnResize(id: string, width: number) {
    widths = { ...widths, [id]: width };
  }

  function handleColumnResizeEnd() {
    onColumnWidthsCommit({ ...widths });
  }

  function handleAutoFitName() {
    if (!tableElement || rows.length === 0) return;
    fittedTreeMode = treeRows !== null;
    nameAutoWidth = computeNameColumnWidth(
      rows,
      tableElement,
      treeRows !== null,
    );
    // Double-click re-fits: drop any manual name width so the measured
    // one takes over, and persist the trimmed map.
    if (widths.name !== undefined) {
      const manual = { ...widths };
      delete manual.name;
      widths = manual;
      onColumnWidthsCommit(manual);
    }
  }

  // --- Row context menu ---
  // The right-clicked row's PID and the cursor position; the menu itself
  // lives here (not per-row) so snapshot refreshes never unmount it.
  let menuPid: number | null = null;
  let menuX = 0;
  let menuY = 0;

  function handleRowContextMenu(process: Process, event: MouseEvent) {
    menuX = event.clientX;
    menuY = event.clientY;
    menuPid = process.pid;
    onHideTooltip();
  }

  function closeContextMenu() {
    menuPid = null;
  }

  // Resolved on every snapshot change: when the right-clicked process leaves
  // the table the menu closes instead of acting on a stale row. In tree view
  // the right-clicked parent is the tree root, exactly as in flat view.
  $: menuProcess =
    menuPid === null
      ? null
      : (rows.find((row) => row.process.pid === menuPid)?.process ?? null);
</script>

<div
  class="table-container"
  style="--row-highlight-duration: {highlightDurationMs}ms"
  use:backToTop
>
  <table bind:this={tableElement}>
    <colgroup>
      {#each visibleColumns as column (column.id)}
        <col
          style={effectiveWidths[column.id] !== undefined
            ? `width: ${effectiveWidths[column.id]}px;`
            : undefined}
        />
      {/each}
      <col style="width: {ACTIONS_WIDTH}px;" />
    </colgroup>
    <TableHeader
      {columns}
      {sortConfig}
      {onToggleSort}
      onColumnResize={handleColumnResize}
      onColumnResizeEnd={handleColumnResizeEnd}
      onAutoFitName={handleAutoFitName}
    />
    <tbody>
      {#each rows as row (row.process.pid)}
        <ProcessRow
          process={row.process}
          {columns}
          isPinned={pinnedProcesses.has(row.process.pid)}
          isJustStarted={justStarted.has(row.process.pid)}
          isSuspended={suspendedPids.has(row.process.pid)}
          treeMode={treeRows !== null}
          depth={row.depth}
          hasChildren={row.hasChildren}
          isExpanded={row.expanded}
          isHighUsage={row.process.cpu_usage > 50 ||
            row.process.memory_usage / (systemStats?.memory_total || 0) > 0.1}
          {onTogglePin}
          {onShowDetails}
          {onRestartProcess}
          {onToggleSuspend}
          {onKillProcess}
          onContextMenu={handleRowContextMenu}
          {onToggleExpand}
          expandPath={row.path}
          familyColor={familyColorFor(row)}
          {onHoverTooltip}
          {onHideTooltip}
        />
      {/each}
    </tbody>
  </table>
</div>

{#if menuProcess}
  <ProcessContextMenu
    process={menuProcess}
    x={menuX}
    y={menuY}
    onKill={(killed) => onKillProcess(killed)}
    onKillApp={(killed) => onKillAppProcess(killed)}
    onClose={closeContextMenu}
  />
{/if}

<style>
  .table-container {
    flex: 1;
    overflow-x: auto;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--surface2) var(--mantle);
  }

  .table-container::-webkit-scrollbar {
    width: 8px;
    height: 8px;
  }

  .table-container::-webkit-scrollbar-track {
    background: var(--mantle);
    border-radius: 4px;
  }

  .table-container::-webkit-scrollbar-thumb {
    background: var(--surface2);
    border-radius: 4px;
    transition: background 0.2s ease;
  }

  .table-container::-webkit-scrollbar-thumb:hover {
    background: var(--surface1);
  }

  .table-container::-webkit-scrollbar-corner {
    background: var(--mantle);
  }

  table {
    width: max-content;
    min-width: 100%;
    table-layout: fixed;
    border-collapse: collapse;
    font-size: 13px;
  }
</style>
