<script>
  let { data = $bindable(), encoded = "", onKindPick } = $props();

  // Only worth showing when it differs from what was typed, which is exactly
  // when a scheme was added and the user should see it.
  let addedScheme = $derived(encoded && encoded !== data.input.trim());
</script>

<label class="input-label" for="payload">Link or text</label>
<textarea
  id="payload"
  class="field js-first-field"
  bind:value={data.input}
  spellcheck="false"
  placeholder="https://example.com"></textarea>

<div class="under">
  <div class="segmented" role="group" aria-label="Content type">
    <button
      type="button"
      aria-pressed={data.kind === "link"}
      onclick={() => onKindPick("link")}>Link</button>
    <button
      type="button"
      aria-pressed={data.kind === "text"}
      onclick={() => onKindPick("text")}>Text</button>
  </div>
  {#if addedScheme}
    <p class="encoded mono" title={encoded}>{encoded}</p>
  {/if}
</div>

<style>
  .under {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 8px;
    min-height: 26px;
  }

  .encoded {
    color: var(--text-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
</style>
