<script lang="ts">
  /**
   * Lightweight SVG line chart for the performance history, styled after
   * the top stats panels: thin grid over the panel background, one stroke
   * per series in a theme color, subtle fill underneath. The buffer is
   * right-aligned: while fewer samples than `capacity` exist, the empty
   * space stays on the left and the newest point touches the right edge.
   */
  export let series: { points: number[]; color: string }[] = [];
  /** Ring buffer size the charts are aligned to. */
  export let capacity = 120;
  /** Fixed upper bound of the scale (e.g. 100 for CPU); auto when null. */
  export let maxHint: number | null = null;

  const VIEW_W = 100;
  const VIEW_H = 40;

  $: scaleMax =
    maxHint ?? Math.max(1e-9, ...series.flatMap((s) => s.points)) * 1.1;

  function polylineOf(points: number[]): string {
    if (points.length === 0) return "";
    const offset = capacity - points.length;
    return points
      .map((value, i) => {
        const x = ((offset + i) / (capacity - 1)) * VIEW_W;
        const y = VIEW_H - (Math.max(0, value) / scaleMax) * VIEW_H;
        return `${x.toFixed(2)},${y.toFixed(2)}`;
      })
      .join(" ");
  }

  // Closed area under the line, only drawn with two points or more
  function areaOf(points: number[]): string {
    const line = polylineOf(points);
    if (!line || points.length < 2) return "";
    const first = line.split(" ")[0].split(",")[0];
    const last = line.split(" ").slice(-1)[0].split(",")[0];
    return `M ${first},${VIEW_H} L ${line.split(" ").join(" L ")} L ${last},${VIEW_H} Z`;
  }

  $: seriesData = series.map((s) => ({
    ...s,
    line: polylineOf(s.points),
    area: areaOf(s.points),
  }));
</script>

<svg
  class="chart"
  viewBox="0 0 {VIEW_W} {VIEW_H}"
  preserveAspectRatio="none"
  aria-hidden="true"
>
  {#each [VIEW_H * 0.25, VIEW_H * 0.5, VIEW_H * 0.75] as gridY}
    <line x1="0" y1={gridY} x2={VIEW_W} y2={gridY} class="grid" />
  {/each}
  {#each seriesData as s (s.color)}
    {#if s.area}
      <path d={s.area} fill={s.color} fill-opacity="0.12" />
    {/if}
    <polyline
      points={s.line}
      fill="none"
      stroke={s.color}
      stroke-width="1.5"
      vector-effect="non-scaling-stroke"
      stroke-linejoin="round"
      stroke-linecap="round"
    />
  {/each}
</svg>

<style>
  .chart {
    display: block;
    width: 100%;
    height: 96px;
  }

  .grid {
    stroke: var(--surface1);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
</style>
