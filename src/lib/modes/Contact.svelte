<script>
  let { data = $bindable() } = $props();

  // Label, key, placeholder, and whether it gets a full-width row.
  const FIELDS = [
    ["First name", "firstName", "Marieke", false],
    ["Last name", "lastName", "van Dijk", false],
    ["Organisation", "org", "Havenlicht Studio", true],
    ["Job title", "title", "Producer", true],
    ["Phone", "phone", "+31 20 555 0134", true],
    ["Email", "email", "marieke@example.com", true],
    ["Website", "url", "https://example.com", true],
  ];
</script>

<label class="input-label" for="firstName">Contact card</label>

<div class="pair">
  {#each FIELDS.slice(0, 2) as [label, key, placeholder]}
    <div class="stack">
      <label class="sub" for={key}>{label}</label>
      <input
        id={key}
        class="field {key === 'firstName' ? 'js-first-field' : ''}"
        bind:value={data[key]}
        spellcheck="false"
        autocomplete="off"
        {placeholder} />
    </div>
  {/each}
</div>

<div class="section">
  <p class="section-caption">Details</p>
  <div class="rows">
    {#each FIELDS.slice(2) as [label, key, placeholder]}
      <div class="row">
        <div class="row-label">{label}</div>
        <input
          id={key}
          class="field wide"
          bind:value={data[key]}
          spellcheck="false"
          autocomplete="off"
          {placeholder} />
      </div>
    {/each}
  </div>
</div>

<p class="hint footnote">
  Saved as a vCard, which both iOS and Android offer to add to contacts. Empty
  fields are left out of the code entirely, so filling in fewer of them makes a
  smaller, easier to scan pattern.
</p>

<style>
  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .stack {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
  }
  .sub {
    font-size: 12px;
    color: var(--text-soft);
  }
  .wide {
    width: 240px;
    flex: 0 0 auto;
  }
  .footnote {
    margin-top: 12px;
  }
</style>
