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
    faNetworkWired,
    faSitemap,
    faList,
  } from "@fortawesome/free-solid-svg-icons";
  import { t } from "$lib/i18n";
  import { overlayStore } from "$lib/stores/overlay";

  export let searchTerm: string;
  export let itemsPerPage: number;
  export let currentPage: number;
  export let totalPages: number;
  export let totalResults: number;
  /** "flat" shows the paginated list; "tree" groups processes by ppid. */
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
</script>

<div class="toolbar">
  <div class="toolbar-content" class:overlay-mode={isAnyOverlayOpen}>
    <div class:hidden={isAnyOverlayOpen && activeOverlayType !== "searchHelp"}>
      <SearchBox bind:searchTerm />
    </div>

    <div class:hidden={isAnyOverlayOpen && activeOverlayType !== "filters"}>
      <FilterToggle bind:filters />
    </div>

    <div class="toolbar-spacer" class:hidden={isAnyOverlayOpen}></div>

    <div class:hidden={isAnyOverlayOpen}>
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
      {#if viewMode === "tree"}
        <!-- Collapse/expand-all toggle for the tree; the icon shows the
             action performed, mirroring the ports modal's control -->
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

    <div class:hidden={isAnyOverlayOpen && activeOverlayType !== "pagination"}>
      {#if viewMode === "tree"}
        <!-- Tree mode flattens hierarchy, so pagination would hide children. -->
        <span class="tree-hint">{$t("tools.treeNoPagination")}</span>
      {:else}
        <PaginationControls
          bind:itemsPerPage
          bind:currentPage
          {totalPages}
          {totalResults}
        />
      {/if}
    </div>
    <div class="toolbar-spacer" class:hidden={isAnyOverlayOpen}></div>

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
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 0 8px;
    /* Wraps to a second row at the window's minimum width instead of
       clipping the right-hand controls */
    min-height: 44px;
    position: relative;
  }

  .toolbar-content > div {
    display: flex;
    align-items: center;
  }

  .toolbar-content :global(.hidden) {
    opacity: 0;
    pointer-events: none;
  }

  .toolbar-spacer {
    flex: 1;
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
  }
</style>
