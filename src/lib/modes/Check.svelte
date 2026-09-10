<script>
  import Icon from "../Icon.svelte";

  let { events, eventId, door, recent, busy, onPickEvent, onScan, onExport } = $props();

  // How long a verdict stays before the field is ready for the next person.
  // Long enough to read across a desk, short enough not to hold up a queue.
  const HOLD_MS = 4000;

  let field = $state(null);
  let entry = $state("");
  let verdict = $state(null);
  let holdTimer;

  const LOOK = {
    admitted: { icon: "check", title: "Admitted", tone: "good" },
    alreadyUsed: { icon: "stop", title: "Already used", tone: "bad" },
    wrongEvent: { icon: "alert", title: "Wrong event", tone: "warn" },
    notGenuine: { icon: "stop", title: "Not genuine", tone: "bad" },
    unreadable: { icon: "alert", title: "Not a ticket", tone: "warn" },
  };

  let look = $derived(verdict ? LOOK[verdict.outcome] : null);

  /**
   * A USB barcode scanner is a keyboard: it types the payload and presses
   * Enter. That makes the whole input path a plain form submit, which is both
   * the most reliable option and what most small venues already own.
   */
  async function submit(e) {
    e?.preventDefault();
    const scanned = entry.trim();
    if (!scanned || busy) return;
    entry = "";
    verdict = await onScan(scanned);
    clearTimeout(holdTimer);
    holdTimer = setTimeout(() => (verdict = null), HOLD_MS);
    field?.focus();
  }

  // The field keeps focus because a scanner types wherever the caret is, and a
  // scan that lands somewhere else is a person waved through unchecked.
  $effect(() => {
    if (eventId) field?.focus();
  });

  function timeOnly(iso) {
    const d = new Date(iso);
    return Number.isNaN(d.getTime())
      ? iso
      : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
</script>

<label class="input-label" for="checkevent">Event</label>
<select
  id="checkevent"
  class="field event-select"
  value={eventId ?? ""}
  onchange={(e) => onPickEvent(e.currentTarget.value || null)}>
  <option value="">Choose an event</option>
  {#each events as ev}
    <option value={ev.id}>{ev.name}</option>
  {/each}
</select>

{#if !eventId}
  <p class="hint spaced">
    Pick the event you are checking tickets for. Its public key decides what
    counts as genuine.
  </p>
{:else}
  <form onsubmit={submit}>
    <label class="input-label scan-label" for="scan">Scan or paste a ticket</label>
    <input
      id="scan"
      class="field js-first-field scan"
      bind:this={field}
      bind:value={entry}
      onblur={() => setTimeout(() => field?.focus(), 0)}
      spellcheck="false"
      autocomplete="off"
      placeholder="Waiting for a scan" />
  </form>

  <div class="verdict {look ? look.tone : 'idle'}">
    {#if verdict && look}
      <Icon name={look.icon} size={22} />
      <div class="what">
        <p class="title">{look.title}</p>
        {#if verdict.outcome === "admitted"}
          <p class="detail">
            Ticket {verdict.serial}{verdict.name ? ` - ${verdict.name}` : ""}
          </p>
        {:else if verdict.outcome === "alreadyUsed"}
          <p class="detail">
            Ticket {verdict.serial}{verdict.name ? ` - ${verdict.name}` : ""}, first
            scanned at {timeOnly(verdict.firstAt)}
          </p>
        {:else if verdict.outcome === "wrongEvent"}
          <p class="detail">This is a genuine ticket for {verdict.event}.</p>
        {:else if verdict.outcome === "notGenuine"}
          <p class="detail">The signature does not match this event's key.</p>
        {:else}
          <p class="detail">{verdict.reason}</p>
        {/if}
      </div>
    {:else}
      <p class="idle-text">Ready</p>
    {/if}
  </div>

  <div class="section">
    <p class="section-caption">This door</p>
    <div class="rows">
      <div class="row">
        <div>
          <div class="row-label">
            {door ? `${door.redeemed} admitted` : "Loading"}
            {#if door && door.issued}
              <span class="of">of {door.issued} issued</span>
            {/if}
          </div>
          <div class="row-helper">
            Counted on this machine only. Two doors checking at once cannot see
            each other, and both would admit the same ticket. Export the log if
            you need to reconcile them afterwards.
          </div>
        </div>
        <button class="text-button" onclick={onExport} disabled={!door?.redeemed}>
          <Icon name="download" /> Export log
        </button>
      </div>
    </div>
  </div>

  {#if recent.length}
    <div class="section">
      <p class="section-caption">Last scans</p>
      <div class="rows">
        {#each recent as r}
          <div class="row recent">
            <span class="dot {LOOK[r.outcome]?.tone ?? 'warn'}"></span>
            <span class="r-what">
              {LOOK[r.outcome]?.title ?? r.outcome}
              {#if r.serial}<span class="r-serial">#{r.serial}</span>{/if}
              {#if r.name}<span class="r-name">{r.name}</span>{/if}
            </span>
            <span class="r-time">{timeOnly(r.at)}</span>
          </div>
        {/each}
      </div>
    </div>
  {/if}
{/if}

<style>
  .event-select {
    width: 260px;
  }
  .spaced {
    margin-top: 12px;
  }
  .scan-label {
    margin-top: 18px;
  }
  .scan {
    font-family: var(--font-mono);
    font-size: 13px;
  }

  /* Deliberately large. This is read at arm's length, across a desk, by
     someone who is also talking to the person in front of them. */
  .verdict {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 14px;
    padding: 18px 20px;
    border-radius: 10px;
    border: 1px solid var(--hairline-strong);
    min-height: 84px;
  }
  .verdict.idle {
    border-style: dashed;
  }
  .verdict.good {
    background: color-mix(in srgb, var(--good) 14%, transparent);
    border-color: var(--good);
    color: var(--good);
  }
  .verdict.warn {
    background: color-mix(in srgb, var(--warn) 14%, transparent);
    border-color: var(--warn);
    color: var(--warn);
  }
  .verdict.bad {
    background: color-mix(in srgb, var(--bad) 14%, transparent);
    border-color: var(--bad);
    color: var(--bad);
  }
  .title {
    font-size: 17px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .detail {
    font-size: 12.5px;
    color: var(--text-soft);
    margin-top: 2px;
  }
  .idle-text {
    font-size: 12px;
    color: var(--text-faint);
  }

  .of {
    color: var(--text-faint);
    font-weight: 400;
  }

  .recent {
    gap: 10px;
    justify-content: flex-start;
    min-height: 34px;
    padding: 6px 0;
  }
  /* One of the few coloured dots in the app, and it earns it: this list is
     scanned for the odd red row, not read line by line. */
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: 0 0 auto;
  }
  .dot.good {
    background: var(--good);
  }
  .dot.warn {
    background: var(--warn);
  }
  .dot.bad {
    background: var(--bad);
  }
  .r-what {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .r-serial {
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
    margin-left: 4px;
  }
  .r-name {
    color: var(--text-soft);
    margin-left: 6px;
  }
  .r-time {
    font-size: 12px;
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
    flex: 0 0 auto;
  }
</style>
