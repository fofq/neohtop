<script lang="ts">
  import {
    AppInfo,
    SearchBox,
    RefreshControls,
    PaginationControls,
    ColumnToggle,
    FilterToggle,
    ToolsMenu,
  } from "$lib/components";
  import Fa from "svelte-fa";
  import {
    faCompressArrowsAlt,
    faExpandArrowsAlt,
    faCodeBranch,
    faNetworkWired,
    faSitemap,
    faList,
    faLayerGroup,
  } from "@fortawesome/free-solid-svg-icons";
  import { t } from "$lib/i18n";
  import { overlayStore } from "$lib/stores/overlay";
  import { settingsStore } from "$lib/stores/settings";
  import type { TreeGrouping } from "$lib/types";

  export let searchTerm: string;
  export let itemsPerPage: number;
  export let currentPage: number;
  export let totalPages: number;
  export let totalResults: number;
  /** "flat" = paginated list; "tree" = grouped view (structure or app
   * grouping, per behavior.treeGrouping). */
  export let viewMode: "flat" | "tree" = "flat";
  /** True while any tree subtree is collapsed; flips the toggle action. */
  export let treeCollapsedAny = false;
  /** Collapses everything (nothing collapsed) or expands everything. */
  export let onToggleTreeCollapse: () => void = () => {};
  export let columns: Array<{
    id: string;
    label: string;
    visible: boolean;
    required?: boolean;
  }>;
  export let refreshRate: number;
  export let isFrozen: boolean;
  export let onShowNetworkPorts: () => void;
  export let onShowFileLockers: () => void;
  export let onShowServices: () => void;
  export let onShowWindows: () => void;
  /** Opens the startup-items manager panel. */
  export let onShowStartupItems: () => void = () => {};
  export let filters: {
    cpu: { operator: string; value: number; enabled: boolean };
    ram: { operator: string; value: number; enabled: boolean };
    runtime: { operator: string; value: number; enabled: boolean };
    status: { values: string[]; enabled: boolean };
  } = {
    cpu: { operator: ">", value: 50, enabled: false },
    ram: { operator: ">", value: 100, enabled: false },
    runtime: { operator: ">", value: 60, enabled: false },
    status: { values: [], enabled: false },
  };

  // Overlay types owned by this toolbar's own dropdowns. Other overlays
  // (e.g. "settings" opened from the title bar) must not hide the toolbar.
  // "theme" is deliberately absent: its dropdown is a self-sufficient
  // portal panel, so the toolbar stays fully visible while it is open.
  const TOOLBAR_OVERLAYS = [
    "pagination",
    "refresh",
    "columns",
    "searchHelp",
    "filters",
    "status",
  ];

  $: isAnyOverlayOpen =
    $overlayStore !== null && TOOLBAR_OVERLAYS.includes($overlayStore);
  $: activeOverlayType = $overlayStore;
  /** Grouping rule of the tree view; persisted in behavior settings. */
  $: treeGrouping = $settingsStore.behavior.treeGrouping;

  function setTreeGrouping(grouping: TreeGrouping) {
    settingsStore.updateConfig({
      behavior: { ...$settingsStore.behavior, treeGrouping: grouping },
    });
  }
</script>

<div class="toolbar">
  <div class="toolbar-content" class:overlay-mode={isAnyOverlayOpen}>
    <div
      class="searchbox-slot"
      class:hidden={isAnyOverlayOpen && activeOverlayType !== "searchHelp"}
    >
      <SearchBox bind:searchTerm />
    </div>

    <div class:hidden={isAnyOverlayOpen && activeOverlayType !== "filters"}>
      <FilterToggle bind:filters />
    </div>

    <!-- The view switch sits right after the filter at a FIXED offset (the
         16px slot margin), so entering or leaving the grouped views never
         moves the tree/flat buttons. Everything to their right is the
         elastic zone below. -->
    <div class="view-group-slot" class:hidden={isAnyOverlayOpen}>
      <div class="view-toggle" role="group" aria-label={$t("tools.viewMode")}>
        <button
          class="view-option"
          class:active={viewMode === "tree"}
          on:click={() => (viewMode = "tree")}
          title={$t("tools.treeView")}
        >
          <Fa icon={faSitemap} />
        </button>
        <button
          class="view-option"
          class:active={viewMode === "flat"}
          on:click={() => (viewMode = "flat")}
          title={$t("tools.flatView")}
        >
          <Fa icon={faList} />
        </button>
      </div>
      <!-- Rendered only in tree view: reserving its space in flat view cost
           ~76px that the 1150px minimum width can no longer spare. The
           pagination/hint zone keeps a constant width, so a view switch
           still moves nothing to the right of the view switch. -->
      {#if viewMode === "tree"}
        <div
          class="view-toggle grouping-toggle"
          role="group"
          aria-label={$t("tools.grouping")}
        >
          <button
            class="view-option"
            class:active={treeGrouping === "structure"}
            on:click={() => setTreeGrouping("structure")}
            title={$t("tools.groupStructure")}
            aria-pressed={treeGrouping === "structure"}
          >
            <Fa icon={faCodeBranch} />
          </button>
          <button
            class="view-option"
            class:active={treeGrouping === "app"}
            on:click={() => setTreeGrouping("app")}
            title={$t("tools.groupApp")}
            aria-pressed={treeGrouping === "app"}
          >
            <Fa icon={faLayerGroup} />
          </button>
        </div>
        <!-- Collapse/expand-all toggle; the icon shows the action performed,
             mirroring the ports modal's control -->
        <button
          class="tree-collapse-toggle"
          on:click={onToggleTreeCollapse}
          title={treeCollapsedAny
            ? $t("tools.expandAll")
            : $t("tools.collapseAll")}
          aria-label={treeCollapsedAny
            ? $t("tools.expandAll")
            : $t("tools.collapseAll")}
        >
          <Fa
            icon={treeCollapsedAny ? faExpandArrowsAlt : faCompressArrowsAlt}
          />
        </button>
      {/if}
    </div>

    <!-- Elastic middle zone: pagination in flat view, the hint in grouped
         views, both stacked in one grid cell and left-aligned. The zone
         flexes to absorb ALL leftover width, so the controls on either
         side of it (view switch at a fixed offset, right-hand cluster at
         the window edge) never move when views or groupings switch. -->
    <div
      class="pagination-zone"
      class:hidden={isAnyOverlayOpen && activeOverlayType !== "pagination"}
    >
      <!-- Grouped views flatten rows into one list, so pagination would
           hide children/members. -->
      <span class="tree-hint" class:hidden={viewMode !== "tree"}>
        {$t("tools.treeNoPagination")}
      </span>
      <div class="pagination-slot" class:hidden={viewMode === "tree"}>
        <PaginationControls
          bind:itemsPerPage
          bind:currentPage
          {totalPages}
          {totalResults}
        />
      </div>
    </div>

    <div class:hidden={isAnyOverlayOpen && activeOverlayType !== "columns"}>
      <ColumnToggle {columns} />
    </div>

    <div class:hidden={isAnyOverlayOpen && activeOverlayType !== "refresh"}>
      <RefreshControls bind:refreshRate bind:isFrozen />
    </div>

    <div class:hidden={isAnyOverlayOpen && activeOverlayType !== "theme"}>
      <ToolsMenu
        {onShowFileLockers}
        {onShowServices}
        {onShowWindows}
        {onShowStartupItems}
      />
      <button
        class="ports-button"
        on:click={onShowNetworkPorts}
        aria-label={$t("ports.ariaOpen")}
        title={$t("ports.title")}
      >
        <span class="icon">
          <Fa icon={faNetworkWired} />
        </span>
      </button>
      <AppInfo />
    </div>
  </div>
</div>

<style>
  .toolbar {
    padding: 8px;
    border-bottom: 1px solid var(--surface0);
    background-color: var(--mantle);
  }

  .toolbar-content {
    display: flex;
    /* Single row, always: at the 1150px window minimum the toolbar just
       fits (the search box and the pagination/hint zone absorb the squeeze
       via flex-shrink), so wrapping to a second row is disabled. */
    flex-wrap: nowrap;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    min-height: 44px;
    position: relative;
  }

  .toolbar-content > div {
    display: flex;
    align-items: center;
    /* Fixed-width controls refuse to shrink; the search slot and the
       pagination zone opt back in below */
    flex-shrink: 0;
  }

  .toolbar-content :global(.hidden) {
    opacity: 0;
    pointer-events: none;
  }

  /* The view switch sits at a fixed 16px offset from the filter, so
     entering/leaving the grouped views never moves the tree/flat buttons. */
  .toolbar-content > .view-group-slot {
    margin-left: 16px;
  }

  /* The search box is the primary shrink candidate: 200px at rest, down
     to 130px at the window minimum. */
  .toolbar-content > .searchbox-slot {
    flex: 0 1 200px;
    min-width: 130px;
  }

  .ports-button {
    height: 28px;
    padding: 0 12px;
    font-size: 12px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
    box-sizing: border-box;
    margin-right: 8px;
  }

  .ports-button:hover {
    background: var(--surface1);
  }

  .ports-button .icon {
    display: inline-flex;
    align-items: center;
    font-size: 12px;
    color: var(--subtext0);
  }

  .view-toggle {
    display: inline-flex;
    align-items: center;
    height: 28px;
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    overflow: hidden;
  }

  .view-option {
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

  .view-option:hover {
    color: var(--text);
    background: var(--surface1);
  }

  .view-option.active {
    color: var(--base);
    background: var(--blue);
  }

  /* Middle zone: pagination (flat view) and the no-pagination hint
     (grouped views) are BOTH mounted and stacked in one grid cell, so the
     zone is exactly as wide as the wider of the two — switching views
     never shifts the neighbouring controls, and the pagination control is
     never squeezed into a guessed fixed width. Needs the parent-scoped
     selector to out-rank `.toolbar-content > div` (flex-shrink: 0). The
     zone is the second shrink candidate: on a too-tight window it clips
     its right edge instead of wrapping the toolbar. */
  /* Elastic middle zone: absorbs ALL leftover width, so the view switch
     (fixed offset after the filter) and the right-hand cluster (window
     edge) stay put across view/grouping switches. Pagination and the
     grouped-view hint are stacked in one grid cell, left-aligned; on a
     too-tight window the zone clips its right edge instead of wrapping.
     Needs the parent-scoped selector to out-rank `.toolbar-content > div`
     (flex-shrink: 0). */
  .toolbar-content > .pagination-zone {
    display: grid;
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    margin: 0 16px;
  }

  .pagination-zone > * {
    grid-area: 1 / 1;
    min-width: 0;
    justify-self: start;
  }

  /* Icon-only grouping rule inside the tree view: its 16px left margin
     (matching the pagination zone's margin-left) lands its left edge
     EXACTLY where the flat view's pagination control starts — filter + 8
     gap + 66 view switch + 8 gap + 16 — so switching views hands the
     pager's spot to the grouping controls instead of reshuffling them. */
  .grouping-toggle {
    margin-left: 16px;
  }

  /* Icon-only collapse/expand-all beside the view switch, matching the
     ports modal's control; only rendered in tree view */
  .tree-collapse-toggle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 28px;
    margin-left: 6px;
    padding: 0;
    font-size: 12px;
    color: var(--subtext0);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .tree-collapse-toggle:hover {
    color: var(--text);
    background: var(--surface1);
  }

  .tree-hint {
    display: inline-flex;
    align-items: center;
    height: 28px;
    padding: 0 4px;
    font-size: 12px;
    color: var(--subtext0);
    white-space: nowrap;
    /* Shrink with an ellipsis instead of forcing the toolbar to wrap to a
       second row when the window gets tight */
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
