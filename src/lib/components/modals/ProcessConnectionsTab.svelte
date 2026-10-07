<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Fa from "svelte-fa";
  import { faNetworkWired, faRefresh } from "@fortawesome/free-solid-svg-icons";
  import { t } from "$lib/i18n";
  import type { PortConnection, Process } from "$lib/types";
  import { formatBytes } from "$lib/utils";

  export let process: Process;

  // The ports panel polls on its own rhythm; this tab owns a light snapshot
  // of its own while it is open (the component unmounts on tab switches, so
  // nothing keeps running behind the user's back)
  const REFRESH_MS = 5000;

  let connections: PortConnection[] | null = null;
  let loading = false;
  let error: string | null = null;
  let refreshTimer: ReturnType<typeof setInterval> | null = null;

  async function loadConnections() {
    if (loading) return;
    loading = true;
    error = null;
    try {
      connections = await invoke<PortConnection[]>("get_network_ports");
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  loadConnections();
  refreshTimer = setInterval(loadConnections, REFRESH_MS);

  onDestroy(() => {
    if (refreshTimer) clearInterval(refreshTimer);
  });

  // Listeners first (the security-relevant end), then the rest in
  // protocol/port order
  function stateRank(state: string): number {
    if (state === "LISTEN") return 0;
    if (state === "ESTABLISHED") return 1;
    return 2;
  }

  $: owned = (connections ?? [])
    .filter((connection) => connection.pid === process.pid)
    .sort(
      (a, b) =>
        stateRank(a.state) - stateRank(b.state) ||
        a.protocol.localeCompare(b.protocol) ||
        a.local_port - b.local_port ||
        a.remote_port - b.remote_port,
    );

  $: hasTraffic = owned.some(
    (connection) => connection.bytes_sent > 0 || connection.bytes_received > 0,
  );

  /** "host:port" with IPv6 brackets; empty endpoints render as "-". */
  function address(addr: string, port: number): string {
    if (!addr && !port) return "-";
    if (addr.includes(":")) return `[${addr}]:${port}`;
    return `${addr}:${port}`;
  }

  /** Localized label of a connection state: looks up `netState.<lowercased
   * state>` and falls back to the raw backend string when no dictionary
   * entry exists (UDP's "-", OS-specific values). The translator is passed
   * in from the template so the cells re-render when the locale changes. */
  function localizedState(
    state: string,
    translate: (key: string) => string,
  ): string {
    if (!state || state === "-") return state;
    const key = `netState.${state.toLowerCase()}`;
    const label = translate(key);
    return label === key ? state : label;
  }

  /** The 6-tuple is the only stable identity a connection has — snapshot
   * refreshes replace the row objects wholesale. */
  function connectionKey(connection: PortConnection): string {
    return [
      connection.protocol,
      connection.local_addr,
      connection.local_port,
      connection.remote_addr,
      connection.remote_port,
      connection.pid,
    ].join("|");
  }
</script>

<div class="connections-content">
  <div class="connections-toolbar">
    <span class="connections-caption">
      <Fa icon={faNetworkWired} />
      {$t("details.connHostedBy", { name: process.name, pid: process.pid })}
    </span>
    <button
      class="connections-refresh"
      on:click={loadConnections}
      disabled={loading}
      aria-label={$t("services.refresh")}
    >
      <span class="refresh-icon">
        {#if loading}
          <span class="spinner"></span>
        {:else}
          <Fa icon={faRefresh} />
        {/if}
      </span>
      <span>{$t("services.refresh")}</span>
    </button>
  </div>

  {#if error}
    <div class="connections-note error">
      <div>{error}</div>
    </div>
  {/if}

  {#if connections === null && !error}
    <div class="connections-status">
      <div class="spinner"></div>
    </div>
  {:else if owned.length === 0}
    <div class="connections-status">{$t("details.connNone")}</div>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>{$t("ports.protocol")}</th>
            <th>{$t("ports.localAddress")}</th>
            <th>{$t("ports.remoteAddress")}</th>
            <th>{$t("ports.state")}</th>
            {#if hasTraffic}
              <th>{$t("ports.received")}</th>
              <th>{$t("ports.sent")}</th>
            {/if}
          </tr>
        </thead>
        <tbody>
          {#each owned as connection (connectionKey(connection))}
            <tr>
              <td class="mono">{connection.protocol}</td>
              <td class="mono">
                {address(connection.local_addr, connection.local_port)}
                {#if connection.state === "LISTEN"}
                  <span class="listen-badge">{$t("ports.stateListen")}</span>
                {/if}
              </td>
              <td class="mono">
                {address(connection.remote_addr, connection.remote_port)}
              </td>
              <td class="mono" title={connection.state}>
                {localizedState(connection.state, $t)}
              </td>
              {#if hasTraffic}
                <td class="mono">
                  {connection.bytes_received > 0
                    ? formatBytes(connection.bytes_received)
                    : "-"}
                </td>
                <td class="mono">
                  {connection.bytes_sent > 0
                    ? formatBytes(connection.bytes_sent)
                    : "-"}
                </td>
              {/if}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .connections-content {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .connections-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .connections-caption {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--subtext0);
  }

  .connections-caption :global(svg) {
    width: 12px;
    height: 12px;
    color: var(--blue);
  }

  .connections-refresh {
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

  .connections-refresh:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface1);
  }

  .connections-refresh:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .connections-status {
    padding: 16px;
    border-radius: 6px;
    background: var(--mantle);
    color: var(--subtext0);
    font-size: 13px;
    display: flex;
    justify-content: center;
  }

  .connections-note {
    padding: 10px 12px;
    border-radius: 6px;
    background: var(--mantle);
    font-size: 12px;
    color: var(--text);
    word-break: break-word;
  }

  .connections-note.error {
    color: var(--red);
    border: 1px solid color-mix(in srgb, var(--red) 40%, transparent);
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

  .listen-badge {
    margin-left: 6px;
    padding: 1px 8px;
    font-size: 11px;
    border-radius: 999px;
    color: var(--yellow);
    background: color-mix(in srgb, var(--yellow) 18%, transparent);
  }

  .refresh-icon {
    display: inline-flex;
    align-items: center;
  }

  .refresh-icon :global(svg) {
    width: 10px;
    height: 10px;
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
