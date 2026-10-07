<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { platform } from "@tauri-apps/plugin-os";
  import Fa from "svelte-fa";
  import {
    faCrosshairs,
    faEyeSlash,
    faRefresh,
    faUpRightFromSquare,
    faWindowMinimize,
    faWindowRestore,
  } from "@fortawesome/free-solid-svg-icons";
  import { backToTop } from "$lib/actions/backToTop";
  import { Modal, SearchInput } from "$lib/components";
  import { t } from "$lib/i18n";
  import { processStore } from "$lib/stores/index";
  import { withElevationHint } from "$lib/utils";
  import type { AppWindow } from "$lib/types";

  export let show = false;
  export let onClose: () => void;

  const isWindows = platform() === "windows";

  let windows: AppWindow[] = [];
  let isLoading = false;
  let error: string | null = null;
  let searchTerm = "";

  type StateFilter = "all" | "visible" | "minimized" | "hidden";

  let stateFilter: StateFilter = "all";
  // Window whose control call is in flight, for per-row feedback
  let showingId: number | null = null;

  // Resolve PIDs against the current process snapshot so rows can jump to
  // the regular process details modal
  $: processByPid = new Map($processStore.processes.map((p) => [p.pid, p]));

  async function loadWindows() {
    if (isLoading) return;
    isLoading = true;
    error = null;
    try {
      windows = await invoke<AppWindow[]>("list_windows");
    } catch (e) {
      error = withElevationHint(e instanceof Error ? e.message : String(e));
    } finally {
      isLoading = false;
    }
  }

  // Reload every time the modal is opened
  $: if (show && isWindows) {
    loadWindows();
  }

  function filterWindows(all: AppWindow[], term: string): AppWindow[] {
    const query = term.trim().toLowerCase();
    if (!query) return all;
    return all.filter(
      (window) =>
        window.title.toLowerCase().includes(query) ||
        window.process_name.toLowerCase().includes(query) ||
        window.pid.toString().includes(query),
    );
  }

  // Raw list → search → state
  $: searchedWindows = filterWindows(windows, searchTerm);

  $: stateFilteredWindows = searchedWindows.filter((window) => {
    if (stateFilter === "all") return true;
    return displayStateOf(window) === stateFilter;
  });

  // Minimized wins over visible: minimized windows usually still carry
  // WS_VISIBLE, so the plain flag alone would mislabel them
  function displayStateOf(window: AppWindow): StateFilter {
    if (window.is_minimized) return "minimized";
    return window.is_visible ? "visible" : "hidden";
  }

  function stateLabel(window: AppWindow): string {
    switch (displayStateOf(window)) {
      case "visible":
        return $t("windows.stateVisible");
      case "minimized":
        return $t("windows.stateMinimized");
      default:
        return $t("windows.stateHidden");
    }
  }

  function stateClass(window: AppWindow): string {
    return `is-${displayStateOf(window)}`;
  }

  // The detail jump closes this panel first: the details modal stacks
  // below it (both z-index 1000, DOM order decides)
  function showDetails(pid: number) {
    const process = processByPid.get(pid);
    if (process) {
      onClose();
      processStore.showProcessDetails(process);
    }
  }

  /** "Bring to front" keeps the dedicated foreground-stealing command;
   * hide/minimize/restore ride the generic ShowWindow switch. */
  async function controlWindow(window: AppWindow, action: string) {
    if (showingId !== null) return;
    showingId = window.id;
    error = null;
    try {
      if (action === "show") {
        await invoke<boolean>("show_window", { id: window.id });
      } else {
        await invoke<boolean>("control_window", { id: window.id, action });
      }
    } catch (e) {
      error = withElevationHint(e instanceof Error ? e.message : String(e));
    } finally {
      showingId = null;
    }
  }
</script>

<Modal {show} title={$t("windows.title")} maxWidth="860px" {onClose}>
  {#if !isWindows}
    <div class="windows-unavailable">
      <Fa icon={faCrosshairs} />
      <p>{$t("windows.unavailable")}</p>
    </div>
  {:else}
    <div class="windows-content">
      <div class="windows-toolbar">
        <div class="windows-search">
          <SearchInput
            bind:value={searchTerm}
            placeholder={$t("windows.searchPlaceholder")}
          />
        </div>
        <button
          class="windows-refresh"
          on:click={() => loadWindows()}
          disabled={isLoading}
          title={$t("windows.refreshAria")}
          aria-label={$t("windows.refreshAria")}
        >
          {#if isLoading}
            <span class="spinner"></span>
          {:else}
            <Fa icon={faRefresh} />
          {/if}
        </button>
      </div>

      <div class="windows-chips">
        <button
          class="chip"
          class:active={stateFilter === "all"}
          on:click={() => (stateFilter = "all")}
        >
          {$t("windows.filterAll")}
        </button>
        <button
          class="chip"
          class:active={stateFilter === "visible"}
          on:click={() => (stateFilter = "visible")}
        >
          {$t("windows.filterVisible")}
        </button>
        <button
          class="chip"
          class:active={stateFilter === "minimized"}
          on:click={() => (stateFilter = "minimized")}
        >
          {$t("windows.filterMinimized")}
        </button>
        <button
          class="chip"
          class:active={stateFilter === "hidden"}
          on:click={() => (stateFilter = "hidden")}
        >
          {$t("windows.filterHidden")}
        </button>
      </div>

      {#if error}
        <div class="windows-error">{error}</div>
      {/if}

      {#if isLoading && windows.length === 0}
        <div class="windows-status">
          <div class="spinner"></div>
        </div>
      {:else if windows.length === 0}
        <div class="windows-status">{$t("windows.empty")}</div>
      {:else}
        <div class="count-row">
          {$t("windows.count", {
            shown: stateFilteredWindows.length,
            total: windows.length,
          })}
        </div>
        <div class="table-wrap" use:backToTop>
          <table>
            <thead>
              <tr>
                <th>{$t("windows.window")}</th>
                <th>{$t("windows.process")}</th>
                <th>{$t("windows.pid")}</th>
                <th>{$t("windows.state")}</th>
                <th class="actions-col"></th>
              </tr>
            </thead>
            <tbody>
              {#each stateFilteredWindows as window (window.id)}
                <tr>
                  <td class="window-title" title={window.title}
                    >{window.title}</td
                  >
                  <td>
                    {#if processByPid.has(window.pid)}
                      <button
                        class="process-link"
                        on:click={() => showDetails(window.pid)}
                        title={$t("windows.showDetails")}
                      >
                        {window.process_name ||
                          processByPid.get(window.pid)?.name ||
                          "-"}
                      </button>
                    {:else}
                      <span class="process-plain"
                        >{window.process_name || "-"}</span
                      >
                    {/if}
                  </td>
                  <td class="mono">{window.pid || "-"}</td>
                  <td>
                    <span class="state-badge {stateClass(window)}"
                      >{stateLabel(window)}</span
                    >
                  </td>
                  <td class="actions-col">
                    <!-- State-dependent action set: visible windows get
                         front/minimize/hide, minimized ones restore/hide,
                         hidden ones just the show switch -->
                    {#if displayStateOf(window) === "minimized"}
                      <button
                        class="row-action"
                        disabled={showingId === window.id}
                        on:click={() => controlWindow(window, "restore")}
                        title={$t("windows.restore")}
                        aria-label={$t("windows.restore")}
                      >
                        {#if showingId === window.id}
                          <span class="spinner"></span>
                        {:else}
                          <Fa icon={faWindowRestore} />
                        {/if}
                      </button>
                      <button
                        class="row-action"
                        disabled={showingId === window.id}
                        on:click={() => controlWindow(window, "hide")}
                        title={$t("windows.hide")}
                        aria-label={$t("windows.hide")}
                      >
                        <Fa icon={faEyeSlash} />
                      </button>
                    {:else if displayStateOf(window) === "hidden"}
                      <button
                        class="row-action"
                        disabled={showingId === window.id}
                        on:click={() => controlWindow(window, "show")}
                        title={$t("windows.showWindow")}
                        aria-label={$t("windows.showWindow")}
                      >
                        {#if showingId === window.id}
                          <span class="spinner"></span>
                        {:else}
                          <Fa icon={faUpRightFromSquare} />
                        {/if}
                      </button>
                    {:else}
                      <button
                        class="row-action"
                        disabled={showingId === window.id}
                        on:click={() => controlWindow(window, "show")}
                        title={$t("windows.showWindow")}
                        aria-label={$t("windows.showWindow")}
                      >
                        <Fa icon={faUpRightFromSquare} />
                      </button>
                      <button
                        class="row-action"
                        disabled={showingId === window.id}
                        on:click={() => controlWindow(window, "minimize")}
                        title={$t("windows.minimize")}
                        aria-label={$t("windows.minimize")}
                      >
                        <Fa icon={faWindowMinimize} />
                      </button>
                      <button
                        class="row-action"
                        disabled={showingId === window.id}
                        on:click={() => controlWindow(window, "hide")}
                        title={$t("windows.hide")}
                        aria-label={$t("windows.hide")}
                      >
                        <Fa icon={faEyeSlash} />
                      </button>
                    {/if}
                    <button
                      class="row-action"
                      disabled={!processByPid.has(window.pid)}
                      on:click={() => showDetails(window.pid)}
                      title={processByPid.has(window.pid)
                        ? $t("windows.showDetails")
                        : $t("windows.notInSnapshot")}
                      aria-label={$t("windows.showDetails")}
                    >
                      <Fa icon={faCrosshairs} />
                    </button>
                  </td>
                </tr>
              {/each}
              {#if stateFilteredWindows.length === 0}
                <tr>
                  <td class="empty-row" colspan="5"
                    >{$t("windows.noResults")}</td
                  >
                </tr>
              {/if}
            </tbody>
          </table>
        </div>
        <div class="crosshair-note">
          <Fa icon={faCrosshairs} />
          <span>{$t("windows.crosshairTodo")}</span>
        </div>
      {/if}
    </div>
  {/if}
</Modal>

<style>
  .windows-content {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .windows-toolbar {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  /* Layout only — the shared SearchInput owns the input's look */
  .windows-search {
    flex: 1;
    min-width: 0;
  }

  /* Icon-only refresh, aligned with the ports panel's toolbar buttons */
  .windows-refresh {
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

  .windows-refresh:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface1);
  }

  .windows-refresh:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .windows-refresh :global(svg) {
    font-size: 11px;
  }

  .windows-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
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

  .count-row {
    font-size: 12px;
    color: var(--subtext0);
  }

  .table-wrap {
    max-height: 55vh;
    overflow: auto;
    border: 1px solid var(--surface0);
    border-radius: 6px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  thead th {
    position: sticky;
    top: 0;
    padding: 8px 10px;
    font-weight: 500;
    text-align: left;
    white-space: nowrap;
    color: var(--subtext0);
    background: var(--mantle);
    border-bottom: 1px solid var(--surface0);
  }

  .actions-col {
    width: 64px;
    text-align: center;
    white-space: nowrap;
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

  .window-title {
    overflow: hidden;
    max-width: 320px;
    text-overflow: ellipsis;
  }

  .mono {
    font-family: monospace;
    font-size: 12px;
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

  .state-badge {
    padding: 1px 8px;
    font-size: 11px;
    border-radius: 999px;
    background: var(--surface1);
  }

  .state-badge.is-visible {
    color: var(--base);
    background: var(--green);
  }

  .state-badge.is-minimized {
    color: var(--base);
    background: var(--yellow);
  }

  .state-badge.is-hidden {
    color: var(--text);
    background: var(--surface1);
  }

  .row-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    margin-left: 4px;
    padding: 0;
    color: var(--subtext0);
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    opacity: 0;
    transition: all 0.15s ease;
  }

  tbody tr:hover .row-action,
  .row-action:focus-visible {
    opacity: 1;
  }

  .row-action:hover:not(:disabled) {
    color: var(--blue);
    background: var(--surface1);
  }

  .row-action:disabled {
    cursor: wait;
    opacity: 0.5;
  }

  .row-action :global(svg) {
    font-size: 11px;
  }

  .empty-row {
    padding: 16px;
    text-align: center;
    color: var(--subtext0);
  }

  .windows-status {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 32px;
    font-size: 13px;
    color: var(--subtext0);
  }

  .windows-unavailable {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: center;
    padding: 40px 16px;
    color: var(--subtext0);
  }

  .windows-unavailable :global(svg) {
    font-size: 28px;
    color: var(--overlay0);
  }

  .windows-unavailable p {
    margin: 0;
    font-size: 13px;
  }

  .windows-error {
    padding: 8px 12px;
    font-size: 13px;
    color: var(--red);
    background: var(--surface0);
    border: 1px solid var(--red);
    border-radius: 6px;
  }

  .crosshair-note {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 0 4px;
    font-size: 11px;
    color: var(--subtext0);
  }

  .crosshair-note :global(svg) {
    font-size: 10px;
    color: var(--overlay0);
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--surface1);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .windows-status .spinner {
    width: 16px;
    height: 16px;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
