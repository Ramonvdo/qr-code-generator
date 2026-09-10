<script>
  import Icon from "../Icon.svelte";

  let { data = $bindable(), busy, onPick, onCopy } = $props();
</script>

<label class="input-label" for="decode">Read a code</label>
<div class="rows">
  <div class="row">
    <div>
      <div class="row-label">{data.fileName || "No image chosen"}</div>
      <div class="row-helper">
        A screenshot or photo of a QR code. Shows what it actually contains,
        which is the one thing a printed code never tells you.
      </div>
    </div>
    <button id="decode" class="btn js-first-field" disabled={busy} onclick={onPick}>
      <Icon name="folder" />
      Choose image
    </button>
  </div>
</div>

{#if data.error}
  <p class="hint err">{data.error}</p>
{/if}

{#if data.results.length}
  <div class="section">
    <p class="section-caption">
      {data.results.length === 1 ? "Contents" : `${data.results.length} codes found`}
    </p>
    <div class="rows">
      {#each data.results as found}
        <div class="row found">
          <p class="value mono">{found}</p>
          <button class="text-button" onclick={() => onCopy(found)}>
            <Icon name="copy" /> Copy
          </button>
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .found {
    align-items: flex-start;
    gap: 12px;
  }
  .value {
    /* Codes can hold a whole vCard, so wrapping beats a scrollbar here. */
    overflow-wrap: anywhere;
    white-space: pre-wrap;
    line-height: 1.5;
    max-height: 180px;
    overflow-y: auto;
    min-width: 0;
    flex: 1;
  }
  .err {
    color: var(--bad);
    margin-top: 10px;
  }
</style>
