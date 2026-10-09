<script lang="ts">
  export let show = false;
  export let maxWidth = "600px";
  export let title: string;
  export let onClose: () => void;
</script>

<!-- Escape closes the modal. svelte:window must sit at the top level, so
     the listener always exists and the handler checks `show`; nested
     modals (a confirm inside a panel) each listen, so Escape dismisses
     both — an accepted trade-off for the rare stacked case. -->
<svelte:window
  on:keydown={(event) => {
    if (show && event.key === "Escape") onClose();
  }}
/>

{#if show}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal-backdrop" on:click={onClose}>
    <div class="modal" on:click|stopPropagation style="--max-width: {maxWidth}">
      <div class="modal-header">
        <h2>{title}</h2>
        <button class="btn-close" on:click={onClose}>×</button>
      </div>
      <div class="modal-content">
        <slot />
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .modal {
    background: var(--base);
    border-radius: 8px;
    width: 100%;
    max-width: var(--max-width);
    max-height: 90vh;
    overflow: auto;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px;
    border-bottom: 1px solid var(--surface0);
  }

  .modal-header h2 {
    margin: 0;
    font-size: 18px;
    color: var(--text);
  }

  .btn-close {
    background: transparent;
    border: none;
    color: var(--overlay0);
    font-size: 24px;
    cursor: pointer;
    padding: 4px 8px;
  }

  .btn-close:hover {
    color: var(--text);
  }

  .modal-content {
    padding: 16px;
  }
</style>
