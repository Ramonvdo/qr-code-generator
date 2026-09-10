<script>
  import Icon from "../Icon.svelte";

  let { data = $bindable(), format, busy, outcome, onExport, onReveal } = $props();

  let count = $derived(data.text.split("\n").filter((l) => l.trim() !== "").length);
</script>

<label class="input-label" for="batchlist">One entry per line</label>
<textarea
  id="batchlist"
  class="field js-first-field list"
  bind:value={data.text}
  spellcheck="false"
  placeholder={"https://example.com/table/1\nhttps://example.com/table/2\nhttps://example.com/table/3"}
></textarea>

<div class="under">
  <div class="segmented" role="group" aria-label="Content type">
    <button type="button" aria-pressed={data.kind === "link"} onclick={() => (data.kind = "link")}>
      Link
    </button>
    <button type="button" aria-pressed={data.kind === "text"} onclick={() => (data.kind = "text")}>
      Text
    </button>
  </div>
  <p class="hint">
    {count === 1 ? "1 entry" : `${count} entries`}
  </p>
</div>

<div class="section">
  <p class="section-caption">Files</p>
  <div class="rows">
    <div class="row">
      <div>
        <div class="row-label">Naming</div>
        <div class="row-helper">
          {#if data.naming === "numbered"}
            Numbered in the order pasted, zero padded so they sort correctly.
          {:else}
            Taken from what each code contains. Duplicates get a suffix rather
            than overwriting each other.
          {/if}
        </div>
      </div>
      <div class="segmented" role="group" aria-label="File naming">
        <button
          type="button"
          aria-pressed={data.naming === "content"}
          onclick={() => (data.naming = "content")}>Contents</button>
        <button
          type="button"
          aria-pressed={data.naming === "numbered"}
          onclick={() => (data.naming = "numbered")}>Numbered</button>
      </div>
    </div>

    <div class="row">
      <div>
        <div class="row-label">Export</div>
        <div class="row-helper">
          Writes one {format.toUpperCase()} per entry into a folder you choose.
        </div>
      </div>
      <button class="btn" disabled={busy || count === 0} onclick={onExport}>
        <Icon name="folder" />
        Choose folder
      </button>
    </div>
  </div>
</div>

{#if outcome}
  <div class="section">
    <p class="section-caption">Result</p>
    <div class="rows">
      <div class="row">
        <div>
          <div class="row-label">
            {outcome.written} of {outcome.written + outcome.failures.length} written
          </div>
          <div class="row-helper">{outcome.directory}</div>
        </div>
        <button class="text-button" onclick={onReveal}>
          <Icon name="folder" /> Show in folder
        </button>
      </div>
    </div>

    {#if outcome.failures.length}
      <!-- Listed in full rather than counted. A batch that drops rows without
           saying which ones is worse than one that refuses to run. -->
      <ul class="failures">
        {#each outcome.failures as f}
          <li>
            <span class="line">Line {f.line}</span>
            <span class="what">{f.content}</span>
            <span class="why">{f.reason}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .list {
    min-height: 150px;
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .under {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 8px;
    min-height: 26px;
  }

  .failures {
    list-style: none;
    margin-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 220px;
    overflow-y: auto;
  }
  .failures li {
    display: grid;
    grid-template-columns: 64px 1fr;
    gap: 2px 10px;
    font-size: 12px;
    line-height: 1.4;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--hairline);
  }
  .line {
    color: var(--bad);
    font-variant-numeric: tabular-nums;
  }
  .what {
    font-family: var(--font-mono);
    overflow-wrap: anywhere;
  }
  .why {
    grid-column: 2;
    color: var(--text-soft);
  }
</style>
