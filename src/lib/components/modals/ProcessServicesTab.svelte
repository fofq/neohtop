<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { platform } from "@tauri-apps/plugin-os";
  import Fa from "svelte-fa";
  import {
    faGears,
    faPlay,
    faRefresh,
    faRotateRight,
    faStop,
  } from "@fortawesome/free-solid-svg-icons";
  import { Modal } from "$lib/components";
  import { t } from "$lib/i18n";
  import { ensureServices, servicesCacheStore } from "$lib/stores/index";
  import { withElevationHint } from "$lib/utils";
  import type { Process, ServiceInfo } from "$lib/types";

  export let process: Process;

  const isWindows = platform() === "windows";

  let confirmAction: {
    service: ServiceInfo;
    action: "start" | "stop" | "restart";
  } | null = null;
  let isControlling = false;
  let controlError: string | null = null;
  let settleTimer: ReturnType<typeof setTimeout> | null = null;

  $: servicesCache = $servicesCacheStore;
  // Only the services hosted by this process (svchost shares its PID with
  // every service it runs; stopped services report PID 0)
  $: hostedServices = (servicesCache.services ?? [])
    .filter((service) => service.pid === process.pid)
    .sort((a, b) => a.name.localeCompare(b.name));

  // Load once when the tab opens; the cache is shared with the hover card
  $: if (isWindows) {
    ensureServices();
  }

  onDestroy(() => {
    if (settleTimer) clearTimeout(settleTimer);
  });

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

  // The SCM state/start-type strings are machine enumerations
  // ("running"/"auto"), not prose — translate them like every other
  // enumerated value and fall back to the raw token for unknown values
  function serviceStatusLabel(
    status: string,
    translate: (key: string) => string,
  ): string {
    const key = `services.status.${status.toLowerCase()}`;
    const label = translate(key);
    return label === key ? status : label;
  }

  function serviceStartTypeLabel(
    startType: string,
    translate: (key: string) => string,
  ): string {
    const key = `services.startType.${startType.toLowerCase()}`;
    const label = translate(key);
    return label === key ? startType : label;
  }

  function actionIcon(action: "start" | "stop" | "restart") {
    switch (action) {
      case "start":
        return faPlay;
      case "stop":
        return faStop;
      case "restart":
        return faRotateRight;
    }
  }

  function actionLabel(action: "start" | "stop" | "restart"): string {
    switch (action) {
      case "start":
        return $t("services.start");
      case "stop":
        return $t("services.stop");
      case "restart":
        return $t("services.restart");
    }
  }

  function askConfirm(
    service: ServiceInfo,
    action: "start" | "stop" | "restart",
  ) {
    controlError = null;
    confirmAction = { service, action };
  }

  async function handleConfirmControl() {
    if (!confirmAction || isControlling) return;
    isControlling = true;
    controlError = null;
    const { service, action } = confirmAction;
    try {
      const accepted =
        action === "restart"
          ? await invoke<boolean>("restart_service", { name: service.name })
          : await invoke<boolean>("control_service", {
              name: service.name,
              action,
            });
      if (!accepted) {
        throw new Error("The service control was not accepted");
      }
      confirmAction = null;
      await ensureServices(true);
      // The state change completes asynchronously on the SCM side, so a
      // second read catches the settled state shortly after
      if (settleTimer) clearTimeout(settleTimer);
      settleTimer = setTimeout(() => {
        ensureServices(true);
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

{#if !isWindows}
  <div class="services-unavailable">
    <Fa icon={faGears} />
    <p>{$t("services.unavailable")}</p>
  </div>
{:else}
  <div class="services-content">
    <div class="services-toolbar">
      <span class="services-caption">
        {$t("services.hostedBy", { name: process.name, pid: process.pid })}
      </span>
      <button
        class="services-refresh"
        on:click={() => ensureServices(true)}
        disabled={servicesCache.loading}
        aria-label={$t("services.refresh")}
      >
        <span class="refresh-icon">
          {#if servicesCache.loading}
            <span class="spinner"></span>
          {:else}
            <Fa icon={faRefresh} />
          {/if}
        </span>
        <span>{$t("services.refresh")}</span>
      </button>
    </div>

    {#if servicesCache.error}
      <div class="control-note error">
        <div>{servicesCache.error}</div>
        <div class="control-hint">{$t("details.adminHint")}</div>
      </div>
    {/if}

    {#if servicesCache.loading && !servicesCache.services}
      <div class="services-status">
        <div class="spinner"></div>
      </div>
    {:else if servicesCache.services && hostedServices.length === 0}
      <div class="services-status">{$t("services.noneHosted")}</div>
    {:else if hostedServices.length > 0}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>{$t("services.name")}</th>
              <th>{$t("services.displayName")}</th>
              <th>{$t("services.status")}</th>
              <th>{$t("services.startType")}</th>
              <th class="actions-col"></th>
            </tr>
          </thead>
          <tbody>
            {#each hostedServices as service (service.name)}
              <tr>
                <td class="mono service-name">{service.name}</td>
                <td>{service.display_name}</td>
                <td>
                  <span class="status-badge {statusClass(service.status)}">
                    {serviceStatusLabel(service.status, $t)}
                  </span>
                </td>
                <td class="mono">
                  {serviceStartTypeLabel(service.start_type, $t)}
                </td>
                <td class="actions-col">
                  {#if service.status === "running"}
                    <button
                      class="row-action"
                      on:click={() => askConfirm(service, "restart")}
                      title={$t("services.restart")}
                      aria-label={$t("services.restart")}
                    >
                      <Fa icon={faRotateRight} />
                    </button>
                    <button
                      class="row-action"
                      on:click={() => askConfirm(service, "stop")}
                      title={$t("services.stop")}
                      aria-label={$t("services.stop")}
                    >
                      <Fa icon={faStop} />
                    </button>
                  {:else if service.status === "stopped"}
                    <button
                      class="row-action"
                      on:click={() => askConfirm(service, "start")}
                      title={$t("services.start")}
                      aria-label={$t("services.start")}
                    >
                      <Fa icon={faPlay} />
                    </button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    {#if controlError}
      <div class="control-note error">
        <div>{controlError}</div>
        <div class="control-hint">{$t("details.adminHint")}</div>
      </div>
    {/if}
  </div>
{/if}

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
          >{serviceStatusLabel(confirmAction.service.status, $t)} ·
          {serviceStartTypeLabel(confirmAction.service.start_type, $t)}</span
        >
      </div>
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
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .services-caption {
    font-size: 12px;
    color: var(--subtext0);
  }

  .services-refresh {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    font-size: 12px;
    color: var(--subtext0);
    background: var(--surface0);
    border: 1px solid var(--surface1);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .services-refresh:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface1);
  }

  .services-refresh:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .services-status {
    padding: 16px;
    border-radius: 6px;
    background: var(--mantle);
    color: var(--subtext0);
    font-size: 13px;
    display: flex;
    justify-content: center;
  }

  .table-wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  th {
    text-align: left;
    padding: 8px;
    color: var(--subtext0);
    font-weight: 500;
    border-bottom: 1px solid var(--surface1);
  }

  td {
    padding: 8px;
    border-bottom: 1px solid var(--surface0);
    color: var(--text);
  }

  tbody tr:last-child td {
    border-bottom: none;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas,
      "Liberation Mono", "Courier New", monospace;
    font-size: 12px;
  }

  .service-name {
    color: var(--subtext1);
  }

  .actions-col {
    text-align: right;
    width: 40px;
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
    padding: 0;
    color: var(--subtext0);
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .row-action:hover {
    color: var(--text);
    background: var(--surface1);
  }

  .row-action :global(svg) {
    width: 10px;
    height: 10px;
  }

  .services-unavailable {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 32px 16px;
    color: var(--subtext0);
    background: var(--mantle);
    border-radius: 8px;
  }

  .services-unavailable :global(svg) {
    width: 20px;
    height: 20px;
  }

  .services-unavailable p {
    margin: 0;
    font-size: 13px;
  }

  /* Error / hint notes, mirroring the details modal control notes */
  .control-note {
    padding: 10px 12px;
    border-radius: 6px;
    background: var(--mantle);
    font-size: 12px;
    color: var(--text);
    word-break: break-word;
  }

  .control-note.error {
    color: var(--red);
    border: 1px solid color-mix(in srgb, var(--red) 40%, transparent);
  }

  .control-note .control-hint {
    margin-top: 6px;
  }

  .control-hint {
    color: var(--subtext0);
    font-size: 12px;
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
    background: var(--mantle);
    padding: 12px;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .service-name {
    color: var(--text);
    font-weight: 500;
    font-size: 13px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas,
      "Liberation Mono", "Courier New", monospace;
  }

  .service-status {
    color: var(--subtext0);
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

  .btn-secondary:hover:not(:disabled) {
    background: var(--surface1);
  }

  .btn-primary {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    font-size: 13px;
    color: var(--base);
    background: var(--blue);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .btn-primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--blue) 90%, white);
  }

  .btn-secondary:disabled,
  .btn-primary:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--surface2);
    border-top-color: var(--text);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
