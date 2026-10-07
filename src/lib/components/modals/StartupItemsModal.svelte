<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Fa from "svelte-fa";
  import {
    faFolderOpen,
    faGears,
    faGlobe,
    faRefresh,
    faRocket,
    faShieldHalved,
    faTrash,
    faTriangleExclamation,
  } from "@fortawesome/free-solid-svg-icons";
  import { backToTop } from "$lib/actions/backToTop";
  import { Modal, SearchInput } from "$lib/components";
  import { t } from "$lib/i18n";
  import { isElevated, settingsStore } from "$lib/stores";
  import { withElevationHint } from "$lib/utils";
  import type { StartupItem } from "$lib/types";

  export let show = false;
  export let onClose: () => void;

  let items: StartupItem[] = [];
  let isLoading = false;
  let error: string | null = null;
  let searchTerm = "";
  let kindFilter: "all" | "registry" | "folder" | "task" | "service" = "all";
  /** Radio state filter: all items, or only enabled / only disabled ones. */
  let stateFilter: "all" | "enabled" | "disabled" = "all";
  /** Item id currently armed for a two-step delete confirmation. */
  let confirmDeleteId: string | null = null;
  let confirmTimer: ReturnType<typeof setTimeout> | null = null;
  /** Id of an in-flight toggle so the switch doesn't flicker. */
  let busyId: string | null = null;

  async function load() {
    isLoading = true;
    error = null;
    try {
      items = await invoke<StartupItem[]>("list_startup_items");
      items.sort((a, b) => a.name.localeCompare(b.name));
    } catch (e) {
      error = withElevationHint(e instanceof Error ? e.message : String(e));
    } finally {
      isLoading = false;
    }
  }

  // Reload every time the panel opens; actions reload on completion
  $: if (show) {
    load();
    disarmConfirm();
  }

  function kindLabel(kind: string): string {
    if (kind === "registry") return $t("startup.kindRegistry");
    if (kind === "folder") return $t("startup.kindFolder");
    if (kind === "service") return $t("startup.kindService");
    return $t("startup.kindTask");
  }

  function kindIcon(kind: string) {
    if (kind === "registry") return faGlobe;
    if (kind === "folder") return faFolderOpen;
    if (kind === "service") return faGears;
    return faRocket;
  }

  function triggerLabel(token: string): string {
    const key = `startup.trigger.${token}`;
    const label = $t(key);
    return label === key ? token : label;
  }

  async function toggle(item: StartupItem) {
    if (busyId !== null) return;
    busyId = item.id;
    error = null;
    try {
      await invoke<boolean>("set_startup_item_enabled", {
        id: item.id,
        enabled: !item.enabled,
      });
      await load();
    } catch (e) {
      error = withElevationHint(e instanceof Error ? e.message : String(e));
    } finally {
      busyId = null;
    }
  }

  function disarmConfirm() {
    if (confirmTimer !== null) clearTimeout(confirmTimer);
    confirmTimer = null;
    confirmDeleteId = null;
  }

  function armConfirm(id: string) {
    disarmConfirm();
    confirmDeleteId = id;
    confirmTimer = setTimeout(() => {
      confirmDeleteId = null;
      confirmTimer = null;
    }, 3000);
  }

  async function remove(item: StartupItem) {
    if (confirmDeleteId !== item.id) {
      armConfirm(item.id);
      return;
    }
    disarmConfirm();
    error = null;
    try {
      await invoke<boolean>("delete_startup_item", { id: item.id });
      await load();
    } catch (e) {
      error = withElevationHint(e instanceof Error ? e.message : String(e));
    }
  }

  onDestroy(disarmConfirm);

  // Windows built-ins (svchost-hosted services, drivers, tasks under
  // \Microsoft\, binaries inside C:\Windows) drown the panel — the
  // default view hides them, the chip brings them back.
  $: hideSystem = $settingsStore.behavior.startupHideSystem ?? true;

  function isSystemNative(item: StartupItem): boolean {
    if (item.kind === "task") {
      // Task locations are task paths ("\Microsoft\Windows\..."); the
      // leading backslash must not defeat the prefix check.
      const path = item.location.toLowerCase().replace(/^\\+/, "");
      return path.startsWith("microsoft");
    }
    const binary = item.command.toLowerCase();
    // "c:\\windows" in source = the literal "c:\windows" — a backslashless
    // "c:windows" would never match a real path and the filter leaked
    // binaries that live directly under C:\Windows
    return (
      binary.includes("c:\\windows") ||
      binary.includes("system32") ||
      binary.includes("systemroot")
    );
  }

  // Search + kind/state filters applied before grouping
  $: query = searchTerm.trim().toLowerCase();
  $: filteredItems = items.filter((item) => {
    if (hideSystem && isSystemNative(item)) {
      return false;
    }
    if (kindFilter !== "all" && item.kind !== kindFilter) {
      return false;
    }
    if (stateFilter === "enabled" && !item.enabled) {
      return false;
    }
    if (stateFilter === "disabled" && item.enabled) {
      return false;
    }
    if (!query) return true;
    return (
      item.name.toLowerCase().includes(query) ||
      item.command.toLowerCase().includes(query) ||
      item.location.toLowerCase().includes(query)
    );
  });

  // Group the filtered list by kind, keeping a stable section order
  $: registryItems = filteredItems.filter((item) => item.kind === "registry");
  $: folderItems = filteredItems.filter((item) => item.kind === "folder");
  $: taskItems = filteredItems.filter((item) => item.kind === "task");
  $: serviceItems = filteredItems.filter((item) => item.kind === "service");
  $: sections = [
    {
      kind: "registry",
      label: $t("startup.kindRegistry"),
      entries: registryItems,
    },
    { kind: "folder", label: $t("startup.kindFolder"), entries: folderItems },
    { kind: "task", label: $t("startup.kindTask"), entries: taskItems },
    {
      kind: "service",
      label: $t("startup.kindService"),
      entries: serviceItems,
    },
  ];
  // A selected kind renders exactly its own section; the "all" view skips
  // empty sections instead of stacking four "nothing found" walls.
  $: visibleSections = sections.filter((section) =>
    kindFilter === "all"
      ? section.entries.length > 0
      : section.kind === kindFilter,
  );

  // Long sections render a preview slice with a "show all" expander so a
  // 257-entry task folder cannot push everything miles below the fold.
  const SECTION_PREVIEW = 25;
  let expandedSections: Set<string> = new Set();
  function expandSection(kind: string) {
    expandedSections = new Set([...expandedSections, kind]);
  }

  async function relaunchAsAdmin() {
    try {
      await invoke<boolean>("restart_as_admin");
    } catch (e) {
      error = withElevationHint(e instanceof Error ? e.message : String(e));
    }
  }
</script>

<Modal
  {show}
  title={$t("startup.title")}
  maxWidth="min(1100px, calc(100vw - 48px))"
  {onClose}
>
  <div class="startup-content">
    {#if !$isElevated}
      <div class="admin-banner">
        <Fa icon={faShieldHalved} />
        <span>{$t("startup.adminRequired")}</span>
        <button class="admin-restart" on:click={relaunchAsAdmin}>
          {$t("settings.elevationAction")}
        </button>
      </div>
    {/if}

    <div class="startup-toolbar">
      <div class="startup-search">
        <SearchInput
          bind:value={searchTerm}
          placeholder={$t("startup.searchPlaceholder")}
        />
      </div>
      <button
        class="chip toolbar-chip"
        class:active={hideSystem}
        on:click={() =>
          settingsStore.updateConfig({
            behavior: {
              ...$settingsStore.behavior,
              startupHideSystem: !hideSystem,
            },
          })}
        title={$t("startup.hideSystem")}
      >
        {$t("startup.hideSystem")}
      </button>
      <button
        class="startup-refresh"
        on:click={load}
        disabled={isLoading}
        title={$t("startup.refreshAria")}
        aria-label={$t("startup.refreshAria")}
      >
        {#if isLoading}
          <span class="spinner"></span>
        {:else}
          <Fa icon={faRefresh} />
        {/if}
      </button>
    </div>

    <!-- Labeled chip groups, mirroring the ports modal's filter row:
         kind and state are two independent radio facets. -->
    <div class="startup-chips" role="group" aria-label={$t("startup.title")}>
      <span class="chip-label">{$t("startup.kindLabel")}</span>
      <button
        class="chip"
        class:active={kindFilter === "all"}
        on:click={() => (kindFilter = "all")}
      >
        {$t("startup.filterAll")}
      </button>
      <button
        class="chip"
        class:active={kindFilter === "registry"}
        on:click={() => (kindFilter = "registry")}
      >
        {$t("startup.kindRegistry")}
      </button>
      <button
        class="chip"
        class:active={kindFilter === "folder"}
        on:click={() => (kindFilter = "folder")}
      >
        {$t("startup.kindFolder")}
      </button>
      <button
        class="chip"
        class:active={kindFilter === "task"}
        on:click={() => (kindFilter = "task")}
      >
        {$t("startup.kindTask")}
      </button>
      <button
        class="chip"
        class:active={kindFilter === "service"}
        on:click={() => (kindFilter = "service")}
      >
        {$t("startup.kindService")}
      </button>
      <span class="chip-divider"></span>
      <span class="chip-label">{$t("startup.stateLabel")}</span>
      <button
        class="chip"
        class:active={stateFilter === "all"}
        on:click={() => (stateFilter = "all")}
      >
        {$t("startup.filterAll")}
      </button>
      <button
        class="chip"
        class:active={stateFilter === "enabled"}
        on:click={() => (stateFilter = "enabled")}
      >
        {$t("startup.stateEnabled")}
      </button>
      <button
        class="chip"
        class:active={stateFilter === "disabled"}
        on:click={() => (stateFilter = "disabled")}
      >
        {$t("startup.stateDisabled")}
      </button>
    </div>

    {#if error}
      <div class="startup-error">
        <Fa icon={faTriangleExclamation} />
        <span>{error}</span>
      </div>
    {/if}

    <div class="startup-list" use:backToTop>
      {#if isLoading && items.length === 0}
        <div class="startup-status">
          <div class="spinner"></div>
        </div>
      {:else if filteredItems.length === 0}
        <div class="startup-status">{$t("startup.noResults")}</div>
      {:else}
        {#each visibleSections as section (section.kind)}
          <div class="section">
            <div class="section-head">
              <span class="section-icon"
                ><Fa icon={kindIcon(section.kind)} /></span
              >
              <span class="section-label">{section.label}</span>
              <span class="section-count">{section.entries.length}</span>
            </div>
            {#if section.entries.length === 0}
              <div class="section-empty">{$t("startup.empty")}</div>
            {:else}
              {#each section.entries.slice(0, expandedSections.has(section.kind) ? section.entries.length : SECTION_PREVIEW) as item (item.id)}
                <div class="item-row" class:disabled={!item.enabled}>
                  <button
                    class="toggle"
                    class:off={!item.enabled}
                    disabled={busyId === item.id}
                    on:click={() => toggle(item)}
                    title={item.enabled
                      ? $t("startup.disable")
                      : $t("startup.enable")}
                    aria-label={item.enabled
                      ? $t("startup.disable")
                      : $t("startup.enable")}
                  >
                    <span class="knob"></span>
                  </button>
                  <div class="item-main">
                    <div class="item-name" title={item.name}>
                      {item.name}
                      {#if item.detail}
                        <span class="item-detail"
                          >{triggerLabel(item.detail)}</span
                        >
                      {/if}
                    </div>
                    {#if item.command}
                      <div class="item-command" title={item.command}>
                        {item.command}
                      </div>
                    {/if}
                    <div class="item-location" title={item.location}>
                      {item.location}
                    </div>
                  </div>
                  {#if item.kind !== "service"}
                    <button
                      class="delete-btn"
                      class:confirming={confirmDeleteId === item.id}
                      on:click={() => remove(item)}
                      title={confirmDeleteId === item.id
                        ? $t("startup.deleteConfirm")
                        : $t("startup.delete")}
                      aria-label={confirmDeleteId === item.id
                        ? $t("startup.deleteConfirm")
                        : $t("startup.delete")}
                    >
                      <Fa icon={faTrash} />
                      {#if confirmDeleteId === item.id}
                        <span>{$t("startup.deleteConfirm")}</span>
                      {/if}
                    </button>
                  {/if}
                </div>
              {/each}
              {#if section.entries.length > SECTION_PREVIEW && !expandedSections.has(section.kind)}
                <button
                  class="section-more"
                  on:click={() => expandSection(section.kind)}
                >
                  {$t("startup.showAll", { count: section.entries.length })}
                </button>
              {/if}
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>
</Modal>

<style>
  /* Mirror the ports modal: the toolbar row stays fixed and only the
     list below scrolls, so filters remain reachable at any depth. */
  .startup-content {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: 78vh;
  }

  .startup-toolbar {
    flex-shrink: 0;
  }

  .startup-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    scrollbar-width: thin;
    scrollbar-color: var(--surface2) var(--mantle);
  }

  .admin-banner {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 12px;
    font-size: 12px;
    color: var(--yellow);
    background: color-mix(in srgb, var(--yellow) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--yellow) 45%, transparent);
    border-radius: 6px;
  }

  .admin-banner :global(svg) {
    flex-shrink: 0;
    font-size: 12px;
  }

  .admin-restart {
    flex-shrink: 0;
    margin-left: auto;
    padding: 3px 10px;
    font-size: 11px;
    color: var(--text);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }

  .admin-restart:hover {
    background: var(--surface1);
    border-color: var(--blue);
  }

  .startup-toolbar {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  /* Layout only — the shared SearchInput owns the input's look */
  .startup-search {
    flex: 1;
    min-width: 0;
  }

  /* The hide-built-in toggle rides in the search row as a view option,
     sized to the input so the row reads as one control strip. */
  .toolbar-chip {
    height: 28px;
  }

  /* Icon-only refresh, aligned with the ports panel's toolbar buttons */
  .startup-refresh {
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

  .startup-refresh:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface1);
  }

  .startup-refresh:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  /* Labeled chip groups, mirroring the ports modal's filter row */
  .startup-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }

  .chip-label {
    font-size: 12px;
    color: var(--subtext0);
  }

  .chip-divider {
    width: 1px;
    height: 16px;
    margin: 0 4px;
    background: var(--surface1);
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
    white-space: nowrap;
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

  .startup-error {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 12px;
    font-size: 13px;
    color: var(--red);
    background: var(--surface0);
    border: 1px solid var(--red);
    border-radius: 6px;
  }

  .startup-status {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 32px;
    font-size: 13px;
    color: var(--subtext0);
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .section-head {
    display: flex;
    gap: 6px;
    align-items: center;
    font-size: 12px;
    font-weight: 600;
    color: var(--subtext0);
  }

  .section-icon :global(svg) {
    font-size: 11px;
  }

  .section-count {
    padding: 0 6px;
    font-weight: 400;
    background: var(--surface0);
    border-radius: 999px;
  }

  .section-empty {
    padding: 6px 10px;
    font-size: 12px;
    color: var(--overlay0);
  }

  .item-row {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 5px 10px;
    background: var(--mantle);
    border-radius: 6px;
  }

  .section-more {
    padding: 4px 10px;
    font-size: 12px;
    color: var(--blue);
    cursor: pointer;
    background: var(--surface0);
    border: none;
    border-radius: 6px;
    text-align: left;
  }

  .section-more:hover {
    background: var(--surface1);
  }

  .item-row.disabled .item-name {
    color: var(--subtext0);
    text-decoration: line-through;
    text-decoration-color: var(--overlay0);
  }

  /* Task-Manager-style enable switch */
  .toggle {
    position: relative;
    flex-shrink: 0;
    width: 30px;
    height: 16px;
    padding: 0;
    background: var(--green);
    border: none;
    border-radius: 999px;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .toggle.off {
    background: var(--surface1);
  }

  .toggle .knob {
    position: absolute;
    top: 2px;
    left: 16px;
    width: 12px;
    height: 12px;
    background: var(--base);
    border-radius: 50%;
    transition: left 0.15s ease;
  }

  .toggle.off .knob {
    left: 2px;
  }

  .toggle:disabled {
    opacity: 0.6;
    cursor: wait;
  }

  .item-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .item-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-detail {
    margin-left: 6px;
    font-size: 11px;
    font-weight: 400;
    color: var(--subtext0);
  }

  .item-command,
  .item-location {
    overflow: hidden;
    font-family: monospace;
    font-size: 11px;
    color: var(--subtext0);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-location {
    font-family: inherit;
    color: var(--overlay0);
  }

  .delete-btn {
    display: inline-flex;
    flex-shrink: 0;
    gap: 6px;
    align-items: center;
    height: 26px;
    padding: 0 8px;
    font-size: 11px;
    color: var(--subtext0);
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }

  .delete-btn:hover {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 10%, transparent);
  }

  .delete-btn.confirming {
    color: var(--base);
    background: var(--red);
  }

  .delete-btn :global(svg) {
    font-size: 11px;
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
