<script lang="ts">
  import { onDestroy } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { invoke } from "@tauri-apps/api/core";
  import { platform } from "@tauri-apps/plugin-os";
  import Fa from "svelte-fa";
  import {
    faCaretRight,
    faGears,
    faLayerGroup,
    faList,
    faPause,
    faPlay,
    faRefresh,
    faStop,
  } from "@fortawesome/free-solid-svg-icons";
  import { Modal } from "$lib/components";
  import { t } from "$lib/i18n";
  import { processStore } from "$lib/stores/index";
  import { withElevationHint } from "$lib/utils";
  import type { Process, ServiceInfo } from "$lib/types";

  export let show = false;
  export let onClose: () => void;

  const isWindows = platform() === "windows";

  let services: ServiceInfo[] = [];
  let isLoading = false;
  let error: string | null = null;
  let searchTerm = "";

  type StatusFilter = "all" | "running" | "stopped" | "paused";
  type SortField = "name" | "display_name" | "status" | "start_type" | "pid";
  type SortDirection = "asc" | "desc";
  type ViewMode = "flat" | "grouped";

  interface HostServiceGroup {
    pid: number;
    name: string;
    services: ServiceInfo[];
  }

  // View mode and sorting; kept in component state so they survive
  // closing and reopening the modal
  let statusFilter: StatusFilter = "all";
  let viewMode: ViewMode = "flat";
  let sortField: SortField | null = null;
  let sortDirection: SortDirection = "asc";
  let expandedGroups: Set<number> = new Set();

  // Resolve PIDs against the current process snapshot so rows can jump to
  // the regular process details modal
  $: processByPid = new Map($processStore.processes.map((p) => [p.pid, p]));

  async function loadServices() {
    if (isLoading) return;
    isLoading = true;
    error = null;
    try {
      services = await invoke<ServiceInfo[]>("list_services");
    } catch (e) {
      error = withElevationHint(e instanceof Error ? e.message : String(e));
    } finally {
      isLoading = false;
    }
  }

  // Reload every time the modal is opened
  $: if (show && isWindows) {
    loadServices();
  }

  onDestroy(() => {
    confirmAction = null;
  });

  function filterServices(all: ServiceInfo[], term: string): ServiceInfo[] {
    const query = term.trim().toLowerCase();
    if (!query) return all;
    return all.filter(
      (service) =>
        service.name.toLowerCase().includes(query) ||
        service.display_name.toLowerCase().includes(query) ||
        service.status.toLowerCase().includes(query) ||
        service.start_type.toLowerCase().includes(query) ||
        service.pid.toString().includes(query),
    );
  }

  // Raw list → search → status → sort → group
  $: searchedServices = filterServices(services, searchTerm);

  $: statusFilteredServices = searchedServices.filter((service) => {
    if (statusFilter === "all") return true;
    if (statusFilter === "paused") {
      return service.status === "paused" || service.status === "pause_pending";
    }
    if (statusFilter === "running") {
      return service.status === "running" || service.status === "start_pending";
    }
    return service.status === "stopped" || service.status === "stop_pending";
  });

  $: filteredServices = sortServices(
    statusFilteredServices,
    sortField,
    sortDirection,
  );

  // The "svchost breakdown" view: services sharing one hosting process
  // collapse under a single group row, which splits what a host process
  // like svchost.exe actually runs
  $: hostGroups = groupByHost(filteredServices, processByPid);

  function sortServices(
    list: ServiceInfo[],
    field: SortField | null,
    direction: SortDirection,
  ): ServiceInfo[] {
    if (field === null) return list;
    const factor = direction === "asc" ? 1 : -1;
    return [...list].sort((a, b) => {
      let cmp = 0;
      switch (field) {
        case "name":
          cmp = a.name.localeCompare(b.name);
          break;
        case "display_name":
          cmp = a.display_name.localeCompare(b.display_name);
          break;
        case "status":
          cmp = a.status.localeCompare(b.status);
          break;
        case "start_type":
          cmp = a.start_type.localeCompare(b.start_type);
          break;
        case "pid":
          cmp = a.pid - b.pid;
          break;
      }
      return cmp * factor;
    });
  }

  function groupByHost(
    list: ServiceInfo[],
    names: Map<number, Process>,
  ): HostServiceGroup[] {
    const groups = new Map<number, HostServiceGroup>();
    for (const service of list) {
      let group = groups.get(service.pid);
      if (!group) {
        group = {
          pid: service.pid,
          name: (service.pid !== 0 && names.get(service.pid)?.name) || "-",
          services: [],
        };
        groups.set(service.pid, group);
      }
      group.services.push(service);
    }
    for (const group of groups.values()) {
      group.services.sort((a, b) => a.name.localeCompare(b.name));
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

  function sortIndicator(field: SortField): string {
    if (sortField !== field) return "↕";
    return sortDirection === "asc" ? "↑" : "↓";
  }

  function sortAria(field: SortField): "ascending" | "descending" | "none" {
    if (sortField !== field) return "none";
    return sortDirection === "asc" ? "ascending" : "descending";
  }

  function statusClass(status: string): string {
    switch (status) {
      case "running":
        return "is-running";
      case "stopped":
        return "is-stopped";
      case "paused":
        return "is-paused";
      default:
        return "is-pending";
    }
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

  // --- Service control (start/stop/pause/resume) with confirmation ---
  type ServiceAction = "start" | "stop" | "pause" | "continue";
  let confirmAction: { service: ServiceInfo; action: ServiceAction } | null =
    null;
  let isControlling = false;
  let controlError: string | null = null;

  // Actions offered per state; the backend re-validates the control
  // against what the service currently accepts
  function actionsOf(service: ServiceInfo): ServiceAction[] {
    switch (service.status) {
      case "running":
        return ["stop", "pause"];
      case "paused":
        return ["continue", "stop"];
      case "stopped":
        return ["start"];
      default:
        return [];
    }
  }

  function actionIcon(action: ServiceAction) {
    switch (action) {
      case "start":
        return faPlay;
      case "stop":
        return faStop;
      case "pause":
        return faPause;
      case "continue":
        return faPlay;
    }
  }

  function actionLabel(action: ServiceAction): string {
    switch (action) {
      case "start":
        return $t("services.start");
      case "stop":
        return $t("services.stop");
      case "pause":
        return $t("services.pause");
      case "continue":
        return $t("services.resume");
    }
  }

  function askConfirm(service: ServiceInfo, action: ServiceAction) {
    controlError = null;
    confirmAction = { service, action };
  }

  async function handleConfirmControl() {
    if (!confirmAction || isControlling) return;
    isControlling = true;
    controlError = null;
    const { service, action } = confirmAction;
    try {
      const accepted = await invoke<boolean>("control_service", {
        name: service.name,
        action,
      });
      if (!accepted) {
        throw new Error("The service control was not accepted");
      }
      confirmAction = null;
      await loadServices();
      // The state change completes asynchronously on the SCM side, so a
      // second read catches the settled state shortly after
      setTimeout(() => {
        if (show) loadServices();
      }, 1500);
    } catch (e) {
      controlError = withElevationHint(
        e instanceof Error ? e.message : String(e),
      );
    } finally {
      isControlling = false;
    }
  }
</script>

<Modal {show} title={$t("services.title")} maxWidth="920px" {onClose}>
  {#if !isWindows}
    <div class="services-unavailable">
      <Fa icon={faGears} />
      <p>{$t("services.unavailable")}</p>
    </div>
  {:else}
    <div class="services-content">
      <div class="services-toolbar">
        <input
          class="services-search"
          type="text"
          placeholder={$t("services.searchPlaceholder")}
          bind:value={searchTerm}
        />
        <div class="view-toggle">
          <button
            class:active={viewMode === "flat"}
            on:click={() => (viewMode = "flat")}
            title={$t("services.flatList")}
          >
            <Fa icon={faList} />
            <span>{$t("services.flatList")}</span>
          </button>
          <button
            class:active={viewMode === "grouped"}
            on:click={() => (viewMode = "grouped")}
            title={$t("services.groupByHost")}
          >
            <Fa icon={faLayerGroup} />
            <span>{$t("services.groupByHost")}</span>
          </button>
        </div>
        <button
          class="services-refresh"
          on:click={() => loadServices()}
          disabled={isLoading}
          aria-label={$t("services.refreshAria")}
        >
          <span class="refresh-icon">
            {#if isLoading}
              <span class="spinner"></span>
            {:else}
              <Fa icon={faRefresh} />
            {/if}
          </span>
          <span class="refresh-label">{$t("services.refresh")}</span>
        </button>
      </div>

      <div class="services-chips">
        <button
          class="chip"
          class:active={statusFilter === "all"}
          on:click={() => (statusFilter = "all")}
        >
          {$t("services.filterAll")}
        </button>
        <button
          class="chip"
          class:active={statusFilter === "running"}
          on:click={() => (statusFilter = "running")}
        >
          {$t("services.filterRunning")}
        </button>
        <button
          class="chip"
          class:active={statusFilter === "stopped"}
          on:click={() => (statusFilter = "stopped")}
        >
          {$t("services.filterStopped")}
        </button>
        <button
          class="chip"
          class:active={statusFilter === "paused"}
          on:click={() => (statusFilter = "paused")}
        >
          {$t("services.filterPaused")}
        </button>
      </div>

      {#if error}
        <div class="services-error">{error}</div>
      {/if}

      {#if isLoading && services.length === 0}
        <div class="services-status">
          <div class="spinner"></div>
        </div>
      {:else if services.length === 0}
        <div class="services-status">{$t("services.empty")}</div>
      {:else}
        <div class="count-row">
          {$t("services.count", {
            shown: filteredServices.length,
            total: services.length,
          })}
        </div>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th aria-sort={sortAria("name")}>
                  <button class="th-sort" on:click={() => toggleSort("name")}>
                    {$t("services.name")}
                    <span
                      class="sort-indicator"
                      class:active={sortField === "name"}
                    >
                      {sortIndicator("name")}
                    </span>
                  </button>
                </th>
                <th aria-sort={sortAria("display_name")}>
                  <button
                    class="th-sort"
                    on:click={() => toggleSort("display_name")}
                  >
                    {$t("services.displayName")}
                    <span
                      class="sort-indicator"
                      class:active={sortField === "display_name"}
                    >
                      {sortIndicator("display_name")}
                    </span>
                  </button>
                </th>
                <th aria-sort={sortAria("status")}>
                  <button class="th-sort" on:click={() => toggleSort("status")}>
                    {$t("services.status")}
                    <span
                      class="sort-indicator"
                      class:active={sortField === "status"}
                    >
                      {sortIndicator("status")}
                    </span>
                  </button>
                </th>
                <th aria-sort={sortAria("start_type")}>
                  <button
                    class="th-sort"
                    on:click={() => toggleSort("start_type")}
                  >
                    {$t("services.startType")}
                    <span
                      class="sort-indicator"
                      class:active={sortField === "start_type"}
                    >
                      {sortIndicator("start_type")}
                    </span>
                  </button>
                </th>
                <th aria-sort={sortAria("pid")}>
                  <button class="th-sort" on:click={() => toggleSort("pid")}>
                    {$t("services.pid")}
                    <span
                      class="sort-indicator"
                      class:active={sortField === "pid"}
                    >
                      {sortIndicator("pid")}
                    </span>
                  </button>
                </th>
                <th class="actions-col"></th>
              </tr>
            </thead>
            <tbody>
              {#if viewMode === "flat"}
                {#each filteredServices as service (service.name)}
                  <tr>
                    <td class="mono service-name">{service.name}</td>
                    <td class="display-name">{service.display_name}</td>
                    <td>
                      <span class="status-badge {statusClass(service.status)}"
                        >{service.status}</span
                      >
                    </td>
                    <td class="mono">{service.start_type}</td>
                    <td class="mono pid-cell">
                      {#if service.pid !== 0 && processByPid.has(service.pid)}
                        <button
                          class="process-link"
                          on:click={() => showDetails(service.pid)}
                          title={$t("services.showDetails")}
                        >
                          {service.pid}
                        </button>
                      {:else}
                        {service.pid || "-"}
                      {/if}
                    </td>
                    <td class="actions-col">
                      {#each actionsOf(service) as action (action)}
                        <button
                          class="row-action"
                          on:click={() => askConfirm(service, action)}
                          title={actionLabel(action)}
                          aria-label={actionLabel(action)}
                        >
                          <Fa icon={actionIcon(action)} />
                        </button>
                      {/each}
                    </td>
                  </tr>
                {/each}
                {#if filteredServices.length === 0}
                  <tr>
                    <td class="empty-row" colspan="6"
                      >{$t("services.noResults")}</td
                    >
                  </tr>
                {/if}
              {:else}
                {#each hostGroups as group (group.pid)}
                  <tr class="group-row">
                    <td colspan="6">
                      <button
                        class="group-head"
                        on:click={() => toggleGroup(group.pid)}
                        aria-expanded={expandedGroups.has(group.pid)}
                      >
                        <span class="group-caret">
                          <Fa icon={faCaretRight} />
                        </span>
                        <span class="group-name">{group.name}</span>
                        <span class="group-pid"
                          >{$t("services.pid")} {group.pid || "-"}</span
                        >
                        <span class="group-count"
                          >{$t("services.hostCount", {
                            count: group.services.length,
                          })}</span
                        >
                      </button>
                    </td>
                  </tr>
                  {#if expandedGroups.has(group.pid)}
                    {#each group.services as service (service.name)}
                      <tr
                        class="child-row"
                        in:fly={{ y: -4, duration: 150 }}
                        out:fade={{ duration: 100 }}
                      >
                        <td class="mono service-name">{service.name}</td>
                        <td class="display-name">{service.display_name}</td>
                        <td>
                          <span
                            class="status-badge {statusClass(service.status)}"
                            >{service.status}</span
                          >
                        </td>
                        <td class="mono">{service.start_type}</td>
                        <td class="mono">
                          {service.pid || "-"}
                        </td>
                        <td class="actions-col">
                          {#each actionsOf(service) as action (action)}
                            <button
                              class="row-action"
                              on:click={() => askConfirm(service, action)}
                              title={actionLabel(action)}
                              aria-label={actionLabel(action)}
                            >
                              <Fa icon={actionIcon(action)} />
                            </button>
                          {/each}
                        </td>
                      </tr>
                    {/each}
                  {/if}
                {/each}
                {#if hostGroups.length === 0}
                  <tr>
                    <td class="empty-row" colspan="6"
                      >{$t("services.noResults")}</td
                    >
                  </tr>
                {/if}
              {/if}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
  {/if}
</Modal>

<Modal
  show={confirmAction !== null}
  title={$t("modal.confirmTitle")}
  maxWidth="420px"
  onClose={() => (confirmAction = null)}
>
  {#if confirmAction}
    <div class="confirm-content">
      <p class="confirm-message">
        {$t("services.confirmMessage", {
          action: actionLabel(confirmAction.action),
          name: confirmAction.service.display_name,
        })}
      </p>
      <div class="service-info">
        <span class="service-name">{confirmAction.service.name}</span>
        <span class="service-status"
          >{confirmAction.service.status} · {confirmAction.service
            .start_type}</span
        >
      </div>
      {#if controlError}
        <p class="confirm-error">{controlError}</p>
      {/if}
      <div class="confirm-actions">
        <button
          class="btn-secondary"
          on:click={() => (confirmAction = null)}
          disabled={isControlling}
        >
          {$t("modal.cancel")}
        </button>
        <button
          class="btn-primary"
          on:click={handleConfirmControl}
          disabled={isControlling}
        >
          {#if isControlling}
            <div class="spinner"></div>
            <span>{$t("services.working")}</span>
          {:else}
            <Fa icon={actionIcon(confirmAction.action)} />
            <span>{actionLabel(confirmAction.action)}</span>
          {/if}
        </button>
      </div>
    </div>
  {/if}
</Modal>

<style>
  .services-content {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .services-toolbar {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .services-search {
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

  .services-search:focus {
    border-color: var(--blue);
  }

  .view-toggle {
    display: inline-flex;
    height: 28px;
    overflow: hidden;
    border: 1px solid var(--surface1);
    border-radius: 6px;
  }

  .view-toggle button {
    display: inline-flex;
    gap: 6px;
    align-items: center;
    padding: 0 10px;
    font-size: 12px;
    color: var(--subtext0);
    background: var(--surface0);
    border: none;
    cursor: pointer;
    transition: all 0.2s ease;
    white-space: nowrap;
  }

  .view-toggle button + button {
    border-left: 1px solid var(--surface1);
  }

  .view-toggle button:hover {
    color: var(--text);
  }

  .view-toggle button.active {
    color: var(--base);
    background: var(--blue);
  }

  .view-toggle button :global(svg) {
    font-size: 11px;
  }

  .services-refresh {
    display: inline-flex;
    gap: 6px;
    align-items: center;
    justify-content: center;
    min-width: 92px;
    height: 28px;
    padding: 0 12px;
    font-size: 12px;
    color: var(--text);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
    white-space: nowrap;
  }

  .services-refresh:hover:not(:disabled) {
    background: var(--surface1);
  }

  .services-refresh:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .refresh-icon {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
  }

  .services-refresh :global(svg) {
    font-size: 11px;
  }

  .services-chips {
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
    max-height: 60vh;
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

  .sort-indicator.active {
    color: var(--blue);
    opacity: 1;
  }

  .actions-col {
    width: 76px;
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

  .service-name {
    font-weight: 600;
    color: var(--blue);
  }

  .display-name {
    overflow: hidden;
    max-width: 280px;
    text-overflow: ellipsis;
  }

  .mono {
    font-family: monospace;
    font-size: 12px;
  }

  .pid-cell {
    padding: 6px 10px;
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

  .status-badge {
    padding: 1px 8px;
    font-size: 11px;
    border-radius: 999px;
    background: var(--surface1);
  }

  .status-badge.is-running {
    color: var(--base);
    background: var(--green);
  }

  .status-badge.is-stopped {
    color: var(--text);
    background: var(--surface1);
  }

  .status-badge.is-paused {
    color: var(--base);
    background: var(--yellow);
  }

  .status-badge.is-pending {
    color: var(--base);
    background: var(--peach);
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

  .row-action:hover {
    color: var(--blue);
    background: var(--surface1);
  }

  .row-action :global(svg) {
    font-size: 11px;
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

  .group-count {
    flex-shrink: 0;
    margin-left: auto;
    padding: 2px 9px;
    font-size: 11px;
    color: var(--text);
    background: var(--surface1);
    border-radius: 999px;
    white-space: nowrap;
  }

  tbody tr.child-row td:first-child {
    padding-left: 24px;
  }

  .empty-row {
    padding: 16px;
    text-align: center;
    color: var(--subtext0);
  }

  .services-status {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 32px;
    font-size: 13px;
    color: var(--subtext0);
  }

  .services-unavailable {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: center;
    padding: 40px 16px;
    color: var(--subtext0);
  }

  .services-unavailable :global(svg) {
    font-size: 28px;
    color: var(--overlay0);
  }

  .services-unavailable p {
    margin: 0;
    font-size: 13px;
  }

  .services-error {
    padding: 8px 12px;
    font-size: 13px;
    color: var(--red);
    background: var(--surface0);
    border: 1px solid var(--red);
    border-radius: 6px;
  }

  .confirm-content {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .confirm-message {
    color: var(--text);
    margin: 0;
    font-size: 14px;
  }

  .service-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px;
    background: var(--mantle);
    border-radius: 6px;
  }

  .service-name {
    color: var(--text);
    font-family: monospace;
    font-size: 13px;
  }

  .service-status {
    color: var(--subtext0);
    font-size: 12px;
  }

  .confirm-error {
    color: var(--red);
    margin: 0;
    font-size: 12px;
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 12px;
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

  .btn-primary {
    padding: 8px 16px;
    font-size: 13px;
    color: var(--base);
    background: var(--blue);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .btn-primary:hover {
    background: color-mix(in srgb, var(--blue) 90%, white);
  }

  .btn-primary:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--surface1);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .btn-primary .spinner {
    border-color: color-mix(in srgb, var(--base) 30%, transparent);
    border-top-color: var(--base);
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
