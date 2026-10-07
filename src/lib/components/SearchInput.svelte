<script lang="ts">
  import { t } from "$lib/i18n";

  /** Shared text-input contract of the main toolbar's search box: same
   * height/radius/focus ring, plus the inline clear button that appears
   * once the field has content. DOM events the parents care about
   * (keydown e.g. Enter-to-search, input e.g. page resets) are forwarded. */
  export let value = "";
  export let placeholder = "";

  $: hasValue = value.trim().length > 0;
</script>

<div class="search-input-wrapper">
  <input
    type="text"
    {placeholder}
    bind:value
    autocomplete="off"
    spellcheck="false"
    class:has-search={hasValue}
    on:keydown
    on:input
  />
  {#if hasValue}
    <button
      class="btn-clear"
      on:click={() => (value = "")}
      title={$t("search.clear")}
      aria-label={$t("search.clear")}
    >
      {$t("search.clear")}
    </button>
  {/if}
</div>

<style>
  .search-input-wrapper {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    min-width: 0;
  }

  input {
    width: 100%;
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--surface1);
    border-radius: 6px;
    font-size: 12px;
    background-color: var(--surface0);
    color: var(--text);
    transition: all 0.2s ease;
    box-sizing: border-box;
  }

  input:hover {
    background-color: var(--surface1);
  }

  input:focus {
    outline: none;
    border-color: var(--blue);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--blue) 25%, transparent);
  }

  input.has-search {
    border-color: var(--blue);
    background: var(--surface1);
  }

  .btn-clear {
    position: absolute;
    right: 4px;
    top: 50%;
    transform: translateY(-50%);
    padding: 4px 8px;
    font-size: 11px;
    color: var(--subtext0);
    background: var(--surface1);
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .btn-clear:hover {
    background: var(--surface2);
    color: var(--text);
  }
</style>
