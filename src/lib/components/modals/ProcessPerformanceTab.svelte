<script lang="ts">
  import {
    faHardDrive,
    faMemory,
    faMicrochip,
  } from "@fortawesome/free-solid-svg-icons";
  import Fa from "svelte-fa";
  import { PanelHeader, PerformanceChart } from "$lib/components";
  import { t } from "$lib/i18n";
  import { formatBytes } from "$lib/utils";
  import type { PerformanceSample, Process } from "$lib/types";

  export let process: Process;
  /** Ring buffer samples of this process (one per polling cycle). */
  export let history: PerformanceSample[] = [];

  $: cpuSeries = [{ points: history.map((p) => p.cpu), color: "var(--blue)" }];
  $: memorySeries = [
    { points: history.map((p) => p.memory), color: "var(--green)" },
  ];
  $: diskSeries = [
    { points: history.map((p) => p.disk_read), color: "var(--teal)" },
    { points: history.map((p) => p.disk_write), color: "var(--peach)" },
  ];

  $: lastSample = history.length > 0 ? history[history.length - 1] : null;
</script>

<div class="perf-grid">
  <div class="chart-card">
    <PanelHeader
      icon={faMicrochip}
      title={$t("details.perf.cpu")}
      usageValue={`${process.cpu_usage.toFixed(1)}%`}
    />
    <PerformanceChart series={cpuSeries} maxHint={100} />
  </div>

  <div class="chart-card">
    <PanelHeader
      icon={faMemory}
      title={$t("details.perf.memory")}
      usageValue={formatBytes(process.memory_usage)}
    />
    <PerformanceChart series={memorySeries} />
  </div>

  <div class="chart-card">
    <PanelHeader
      icon={faHardDrive}
      title={$t("details.perf.disk")}
      usageValue={lastSample
        ? $t("details.perf.diskSummary", {
            read: formatBytes(lastSample.disk_read),
            write: formatBytes(lastSample.disk_write),
          })
        : null}
    />
    <div class="disk-legend">
      <span class="legend-item">
        <span class="legend-swatch read"></span>{$t("details.perf.diskRead")}
      </span>
      <span class="legend-item">
        <span class="legend-swatch write"></span>{$t("details.perf.diskWrite")}
      </span>
    </div>
    <PerformanceChart series={diskSeries} />
  </div>
</div>

{#if history.length < 2}
  <div class="perf-hint">
    {$t("details.perf.collecting", { count: history.length })}
  </div>
{/if}

<style>
  .perf-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: 16px;
  }

  .chart-card {
    background: var(--mantle);
    border-radius: 8px;
    padding: 12px;
  }

  .disk-legend {
    display: flex;
    gap: 16px;
    margin-bottom: 6px;
  }

  .legend-item {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--subtext0);
  }

  .legend-swatch {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }

  .legend-swatch.read {
    background: var(--teal);
  }

  .legend-swatch.write {
    background: var(--peach);
  }

  .perf-hint {
    margin-top: 12px;
    padding: 10px 12px;
    border-radius: 6px;
    background: var(--mantle);
    font-size: 12px;
    color: var(--subtext0);
  }
</style>
