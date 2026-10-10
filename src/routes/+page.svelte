<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { debounce } from "$lib/utils";
  import {
    StatsBar,
    ToolBar,
    TitleBar,
    ProcessTable,
    ProcessHoverCard,
    ProcessDetailsModal,
    KillProcessModal,
    RestartProcessModal,
    NetworkPortsModal,
    FileLockersModal,
    ServicesModal,
    WindowsModal,
    StartupItemsModal,
  } from "$lib/components/index";
  import {
    themeStore,
    settingsStore,
    processStore,
    initElevation,
  } from "$lib/stores/index";
  import { initLocale, t } from "$lib/i18n";
  import { column_definitions } from "$lib/definitions/columns";
  import {
    filterProcesses,
    sortProcesses,
    withAncestors,
    buildTreeRows,
    buildAppRows,
    buildSearchTiers,
    type SearchTier,
  } from "$lib/utils";
  import type { Process, ProcessTreeRow, TreeGrouping } from "$lib/types";

  $: ({
    processes,
    systemStats,
    error,
    searchTerm,
    isLoading,
    currentPage,
    pinnedProcesses,
    justStarted,
    suspendedPids,
    selectedProcess,
    showInfoModal,
    showConfirmModal,
    processToKill,
    isKilling,
    killAppCount,
    notice,
    showRestartModal,
    processToRestart,
    isRestarting,
    isFrozen,
    sortConfig,
  } = $processStore);

  let intervalId: ReturnType<typeof setInterval>;
  let lastProcessCount = 0;
  let cachedFilteredProcesses: Process[] = [];
  let cachedSortedProcesses: Process[] = [];
  // Search relevance per pid (0 = name hit, 1 = command-line/PID-only
  // hit); empty while no search is active. Feeds sortProcesses so name
  // hits lead the flat list regardless of the sorted column.
  let searchTierMap = new Map<number, SearchTier>();

  // "flat" = paginated list (default); "tree" = grouped view, either by
  // ppid lineage or by application family (behavior.treeGrouping).
  let viewMode: "flat" | "tree" = "flat";
  /**
   * Subtrees collapsed in the structure grouping, keyed by the
   * root-to-node name chain (ProcessTreeRow.path). Name paths survive the
   * PID churn of short-lived child processes, so a collapsed group stays
   * collapsed across refreshes.
   */
  let collapsedPaths = new Set<string>();
  /** Collapsed app-group leaders, keyed by the stable "app:session|exe"
   * identity — never the "Name (N)" display name, whose live count would
   * re-expand collapsed groups on every member churn. */
  let appCollapsedPaths = new Set<string>();
  /** Grouping rule of the tree view (persisted behavior setting). */
  $: treeGrouping = $settingsStore.behavior.treeGrouping;

  function toggleExpand(path: string) {
    // Each grouping tracks its own collapse set; app-group rows carry the
    // stable "app:..." identity, structure rows the root-to-node chain
    const next = new Set(
      treeGrouping === "app" ? appCollapsedPaths : collapsedPaths,
    );
    if (next.has(path)) {
      next.delete(path);
    } else {
      next.add(path);
    }
    if (treeGrouping === "app") {
      appCollapsedPaths = next;
    } else {
      collapsedPaths = next;
    }
    resetTreeIdleTimer();
  }

  // Collapse-all needs every expandable path, including ones currently
  // hidden behind collapsed nodes, so it walks a fully expanded tree.
  function collapseAllTree() {
    if (treeGrouping === "app") {
      const full = buildAppRows(
        cachedFilteredProcesses,
        sortConfig,
        new Set<string>(),
        searchTierMap,
      );
      appCollapsedPaths = new Set(
        full.filter((row) => row.hasChildren).map((row) => row.path),
      );
      return;
    }
    const full = buildTreeRows(
      visibleTreeProcesses,
      sortConfig,
      new Set<string>(),
      pinnedProcesses,
      searchTierMap,
    );
    collapsedPaths = new Set(
      full.filter((row) => row.hasChildren).map((row) => row.path),
    );
  }

  function expandAllTree() {
    if (treeGrouping === "app") {
      appCollapsedPaths = new Set();
    } else {
      collapsedPaths = new Set();
    }
  }

  function toggleTreeCollapse() {
    const anyCollapsed =
      treeGrouping === "app"
        ? appCollapsedPaths.size > 0
        : collapsedPaths.size > 0;
    if (anyCollapsed) {
      expandAllTree();
    } else {
      collapseAllTree();
    }
  }

  // Idle auto-collapse: an open grouped view folds itself after N seconds
  // without any expand/collapse interaction (0 = off). Leaving the grouped
  // views cancels the clock.
  let treeIdleTimer: ReturnType<typeof setTimeout> | null = null;
  $: treeAutoCollapseSeconds = $settingsStore.behavior.treeAutoCollapseSeconds;

  function cancelTreeIdleTimer() {
    if (treeIdleTimer !== null) {
      clearTimeout(treeIdleTimer);
      treeIdleTimer = null;
    }
  }

  function resetTreeIdleTimer() {
    cancelTreeIdleTimer();
    if (viewMode !== "tree" || treeAutoCollapseSeconds <= 0) return;
    treeIdleTimer = setTimeout(() => {
      treeIdleTimer = null;
      collapseAllTree();
    }, treeAutoCollapseSeconds * 1000);
  }

  // What a grouped view starts as (collapsed by default, per the settings).
  // Applied only when the view, the grouping or the setting CHANGES — never
  // on manual collapse/expand, whose sets stay untouched.
  function applyTreeExpansionDefault(
    defaultExpanded: boolean,
    grouping: TreeGrouping,
  ) {
    if (defaultExpanded) {
      expandAllTree();
    } else {
      collapseAllTree();
    }
    resetTreeIdleTimer();
  }

  // Initialize filters object for the new FilterToggle
  let filters = {
    cpu: { operator: ">", value: 50, enabled: false },
    ram: { operator: ">", value: 100, enabled: false },
    runtime: { operator: ">", value: 60, enabled: false },
    status: { values: [], enabled: false },
  };

  let showNetworkPorts = false;
  let showFileLockers = false;
  let showServices = false;
  let showWindows = false;
  let showStartupItems = false;

  /** Rich row tooltip; driven via show()/hide() from the table events. */
  let hoverCard: ProcessHoverCard | null = null;

  $: columns = column_definitions.map((col) => ({
    ...col,
    visible:
      col.required ||
      ($settingsStore.appearance.columnVisibility[col.id] ?? col.visible),
  }));
  $: itemsPerPage = $settingsStore.behavior.itemsPerPage;
  $: refreshRate = $settingsStore.behavior.refreshRate;

  // Throttled filtering to reduce CPU usage; the tier map is rebuilt with
  // the same terms so filter and rank stay consistent (must land before
  // the reactive re-sort below reads it, which the shared flush guarantees)
  const debouncedFilter = debounce(() => {
    cachedFilteredProcesses = filterProcesses(processes, searchTerm, filters);
    searchTierMap = buildSearchTiers(cachedFilteredProcesses, searchTerm);
  }, 100);

  // Only recalculate filtering when inputs actually change
  $: if (
    processes.length !== lastProcessCount ||
    searchTerm ||
    Object.values(filters).some((f) => f.enabled)
  ) {
    lastProcessCount = processes.length;
    debouncedFilter();
  } else if (
    processes.length === lastProcessCount &&
    !searchTerm &&
    !Object.values(filters).some((f) => f.enabled)
  ) {
    // No filters applied, use all processes directly; drop the stale tier
    // map without reassigning an already-empty one (would retrigger the
    // sort below on every refresh tick for nothing)
    cachedFilteredProcesses = processes;
    if (searchTierMap.size > 0) searchTierMap = new Map();
  }

  // Cache sorted results to avoid re-sorting unchanged data; pinned
  // processes float to the top in pin order ahead of the sort field, and
  // while a search is active name hits rank ahead of command-line/PID
  // hits within that same order
  $: if (cachedFilteredProcesses && sortConfig) {
    cachedSortedProcesses = sortProcesses(
      cachedFilteredProcesses,
      sortConfig,
      pinnedProcesses,
      searchTierMap,
    );
  } else {
    cachedSortedProcesses = cachedFilteredProcesses;
  }

  // Floor at 1: an empty filtered list would otherwise render the sick
  // "page 1 / 0" indicator (and the pager buttons still no-op at 1/1)
  $: totalPages = Math.max(
    1,
    Math.ceil(cachedFilteredProcesses.length / itemsPerPage),
  );
  $: paginatedProcesses = cachedSortedProcesses.slice(
    (currentPage - 1) * itemsPerPage,
    currentPage * itemsPerPage,
  );

  // Grouped views: search/filter hits keep their ancestor chain visible
  // (structure grouping), sort order applies to siblings within each level
  // (pinned processes are hoisted to the top of the root level), search
  // tiers rank every level, and pagination is off in both groupings.
  let treeRows: ProcessTreeRow[] | null = null;
  $: visibleTreeProcesses =
    viewMode === "tree" && treeGrouping === "structure"
      ? withAncestors(cachedFilteredProcesses, processes)
      : [];

  // Entering a grouped view applies the preferred default grouping (the
  // "when entering the tree view" setting); leaving just records the
  // transition. Matching on the TRANSITION — not the state — keeps manual
  // grouping switches inside the view untouched.
  let prevViewMode: "flat" | "tree" = "flat";
  $: {
    if (viewMode === "tree" && prevViewMode !== "tree") {
      prevViewMode = "tree";
      const preferred = $settingsStore.behavior.treeDefaultGrouping;
      if (treeGrouping !== preferred) {
        settingsStore.updateConfig({
          behavior: { ...$settingsStore.behavior, treeGrouping: preferred },
        });
      }
    } else if (viewMode !== "tree" && prevViewMode !== "flat") {
      prevViewMode = "flat";
    }
  }

  // Grouped views START the way the settings dictate (collapsed by
  // default). Fires only when the view, the grouping or the setting
  // CHANGES — manual collapse/expand never touches those. The idle timer
  // is deliberately NOT a dependency (resetting it must never re-fold the
  // rows the user just expanded). Positioned after visibleTreeProcesses
  // and before treeRows so a view switch computes the collapse sets in the
  // SAME update pass that renders the rows — no expanded flash first.
  $: if (viewMode === "tree") {
    applyTreeExpansionDefault(
      $settingsStore.behavior.treeDefaultExpanded,
      treeGrouping,
    );
  } else {
    cancelTreeIdleTimer();
  }

  $: treeRows =
    viewMode === "tree"
      ? treeGrouping === "app"
        ? buildAppRows(
            cachedFilteredProcesses,
            sortConfig,
            appCollapsedPaths,
            searchTierMap,
          )
        : buildTreeRows(
            visibleTreeProcesses,
            sortConfig,
            collapsedPaths,
            pinnedProcesses,
            searchTierMap,
          )
      : null;

  // The current page is meaningless while paging is off; reset it so
  // switching back to flat view never lands on an empty page.
  $: if (viewMode === "flat" && currentPage !== 1 && currentPage > totalPages) {
    currentPage = 1;
  }

  $: {
    if (searchTerm || itemsPerPage) {
      currentPage = 1;
    }
  }

  $: {
    if (intervalId) clearInterval(intervalId);
    if (!isFrozen) {
      // Use adaptive refresh rate - slightly slower for 1s to reduce CPU usage
      const adaptiveRefreshRate = refreshRate === 1000 ? 1500 : refreshRate;
      intervalId = setInterval(() => {
        processStore.getProcesses();
      }, adaptiveRefreshRate);
    }
  }

  // --- Port watch ---
  // Polls the backend's listening-port snapshot and toasts when one of the
  // user's watched ports gains a listener. The timer follows the watched
  // list (joined to a string so only real list changes restart it) and the
  // first tick after a restart only seeds the seen-set, so ports that are
  // already listening when the app starts never toast.
  const PORT_WATCH_INTERVAL_MS = 5000;
  let watchTimer: ReturnType<typeof setInterval> | null = null;
  let watchSeen: Set<number> = new Set();
  let watchSeeded = false;

  $: watchedPortList = ($settingsStore.behavior.portsWatched ?? []).slice();
  $: {
    if (watchTimer !== null) clearInterval(watchTimer);
    watchTimer = null;
    if (watchedPortList.length > 0) {
      watchSeeded = false;
      watchTimer = setInterval(checkWatchedPorts, PORT_WATCH_INTERVAL_MS);
      checkWatchedPorts();
    }
  }

  async function checkWatchedPorts() {
    try {
      const ports: number[] = await invoke("get_listening_ports");
      const now = new Set(ports);
      if (watchSeeded) {
        const translate = get(t);
        for (const port of watchedPortList) {
          if (now.has(port) && !watchSeen.has(port)) {
            // Prefer the user's own name for the port when one is set
            const label =
              get(settingsStore).behavior.portLabels?.[String(port)] ?? "";
            processStore.showNotice(
              label
                ? translate("ports.watchAppearedLabeled", { port, label })
                : translate("ports.watchAppeared", { port }),
            );
          }
        }
      }
      watchSeeded = true;
      watchSeen = now;
    } catch {
      // Keep the previous seen-set on transient backend failures so a
      // hiccup doesn't mask (or duplicate) the next real transition
    }
  }

  onMount(async () => {
    try {
      await processStore.getProcesses();
    } catch (error) {
      console.error("Failed to load processes:", error);
    } finally {
      processStore.setIsLoading(false);
    }

    settingsStore.init();
    initLocale(get(settingsStore).language);
    themeStore.init();
    initElevation();
  });

  onDestroy(() => {
    if (intervalId) clearInterval(intervalId);
    if (watchTimer !== null) clearInterval(watchTimer);
    if (treeIdleTimer !== null) clearTimeout(treeIdleTimer);
  });
</script>

{#if isLoading}
  <div class="loading-container">
    <div class="loading-content">
      <img src="128x128.png" alt="NeoHtop Logo" class="logo" />
    </div>
  </div>
{:else}
  <div class="app-container">
    <TitleBar />
    <main>
      {#if systemStats}
        <StatsBar {systemStats} />
      {/if}

      <ToolBar
        bind:searchTerm={$processStore.searchTerm}
        bind:itemsPerPage
        bind:currentPage={$processStore.currentPage}
        bind:refreshRate
        bind:isFrozen={$processStore.isFrozen}
        bind:filters
        bind:viewMode
        treeCollapsedAny={treeGrouping === "app"
          ? appCollapsedPaths.size > 0
          : collapsedPaths.size > 0}
        onToggleTreeCollapse={toggleTreeCollapse}
        {totalPages}
        totalResults={cachedFilteredProcesses.length}
        bind:columns
        onShowNetworkPorts={() => (showNetworkPorts = true)}
        onShowFileLockers={() => (showFileLockers = true)}
        onShowServices={() => (showServices = true)}
        onShowWindows={() => (showWindows = true)}
        onShowStartupItems={() => (showStartupItems = true)}
      />

      {#if error}
        <div class="alert">{error}</div>
      {:else if notice}
        <div class="notice">{notice}</div>
      {/if}

      <ProcessTable
        processes={paginatedProcesses}
        {columns}
        {systemStats}
        {sortConfig}
        {pinnedProcesses}
        {justStarted}
        {suspendedPids}
        {treeRows}
        highlightDurationMs={$settingsStore.appearance.highlighting.durationMs}
        columnWidths={$settingsStore.appearance.columnWidths}
        onColumnWidthsCommit={(widths) =>
          settingsStore.updateConfig({
            appearance: { ...$settingsStore.appearance, columnWidths: widths },
          })}
        onToggleSort={processStore.toggleSort}
        onTogglePin={processStore.togglePin}
        onShowDetails={(process) => {
          hoverCard?.hide();
          processStore.showProcessDetails(process);
        }}
        onRestartProcess={processStore.confirmRestartProcess}
        onToggleSuspend={processStore.toggleSuspend}
        onKillProcess={processStore.confirmKillProcess}
        onKillAppProcess={processStore.confirmKillAppProcess}
        onToggleExpand={toggleExpand}
        onHoverTooltip={(process, event) =>
          hoverCard?.show(process, event.clientX, event.clientY)}
        onHideTooltip={() => hoverCard?.hide()}
      />
    </main>
  </div>
{/if}

<ProcessHoverCard bind:this={hoverCard} />

<ProcessDetailsModal
  show={showInfoModal}
  process={selectedProcess}
  {processes}
  onClose={processStore.closeProcessDetails}
  onShowDetails={processStore.showProcessDetails}
/>

<KillProcessModal
  show={showConfirmModal}
  process={processToKill}
  appCount={killAppCount}
  {isKilling}
  onClose={processStore.closeConfirmKill}
  onConfirm={processStore.handleConfirmKill}
/>

<RestartProcessModal
  show={showRestartModal}
  process={processToRestart}
  {isRestarting}
  onClose={processStore.closeConfirmRestart}
  onConfirm={processStore.handleConfirmRestart}
/>

<NetworkPortsModal
  show={showNetworkPorts}
  onClose={() => (showNetworkPorts = false)}
/>

<FileLockersModal
  show={showFileLockers}
  onClose={() => (showFileLockers = false)}
/>

<ServicesModal show={showServices} onClose={() => (showServices = false)} />

<WindowsModal show={showWindows} onClose={() => (showWindows = false)} />

<StartupItemsModal
  show={showStartupItems}
  onClose={() => (showStartupItems = false)}
/>

<style>
  :global(:root) {
    --base: #1e1e2e;
    --mantle: #181825;
    --crust: #11111b;
    --text: #cdd6f4;
    --subtext0: #a6adc8;
    --subtext1: #bac2de;
    --surface0: #313244;
    --surface1: #45475a;
    --surface2: #585b70;
    --overlay0: #6c7086;
    --overlay1: #7f849c;
    --blue: #89b4fa;
    --lavender: #b4befe;
    --sapphire: #74c7ec;
    --sky: #89dceb;
    --red: #f38ba8;
    --maroon: #eba0ac;
    --peach: #fab387;
    --yellow: #f9e2af;
    --green: #a6e3a1;
    --teal: #94e2d5;
  }

  :global(body) {
    margin: 0;
    padding: 0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial,
      "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", "Noto Sans CJK SC",
      sans-serif, "Apple Color Emoji", "Segoe UI Emoji";
    background-color: var(--base);
    color: var(--text);
    -webkit-font-smoothing: antialiased;
    overflow: hidden;
    user-select: none;
  }

  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: min-content;
    overflow: hidden;
  }

  .app-container {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }

  .alert {
    margin: 8px;
    padding: 8px 12px;
    background-color: var(--surface0);
    border: 1px solid var(--red);
    border-radius: 6px;
    color: var(--red);
    font-size: 13px;
  }

  .notice {
    margin: 8px;
    padding: 8px 12px;
    background-color: var(--surface0);
    border: 1px solid var(--green);
    border-radius: 6px;
    color: var(--green);
    font-size: 13px;
  }

  .loading-container {
    width: 100vw;
    height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: linear-gradient(135deg, var(--base) 0%, var(--mantle) 100%);
    position: relative;
    overflow: hidden;
  }

  .loading-content {
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2;
  }

  .logo {
    width: 128px;
    height: 128px;
    filter: drop-shadow(0 0 5px var(--text)) drop-shadow(0 0 10px var(--text))
      drop-shadow(0 0 20px var(--blue)) drop-shadow(0 0 40px var(--blue));
    animation: neonPulse 2s ease-in-out infinite;
  }

  @keyframes neonPulse {
    0%,
    100% {
      filter: drop-shadow(0 0 5px var(--text)) drop-shadow(0 0 10px var(--text))
        drop-shadow(0 0 20px var(--blue)) drop-shadow(0 0 40px var(--blue));
    }
    50% {
      filter: drop-shadow(0 0 10px var(--text))
        drop-shadow(0 0 20px var(--text)) drop-shadow(0 0 40px var(--blue))
        drop-shadow(0 0 80px var(--blue));
    }
  }
</style>
