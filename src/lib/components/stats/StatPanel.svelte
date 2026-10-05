<script lang="ts">
  export let title: string;
  export let flex = 1;
</script>

<div class="stat-panel" style="--flex: {flex}">
  <h3 class="panel-title">{title}</h3>
  <div class="panel-content">
    <slot />
  </div>
</div>

<style>
  .stat-panel {
    flex: var(--flex);
    min-width: 180px;
    background: var(--mantle);
    border-radius: 8px;
    padding: 12px;
  }

  .panel-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--subtext0);
    margin: 0 0 12px 0;
  }

  .panel-content {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
</style>
