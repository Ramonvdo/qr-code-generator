<script>
  import Icon from "../Icon.svelte";
  import Layout from "../Layout.svelte";

  let {
    events,
    event = $bindable(),
    buyerFile,
    required,
    verdict,
    busy,
    outcome,
    onPickEvent,
    onNewEvent,
    onDuplicate,
    onPickTemplate,
    onPickBuyers,
    onRemap,
    onClearBuyers,
    onExampleCsv,
    onIssue,
    onReveal,
    sendable,
    sendReady,
    sending,
    onSend,
    onCancelSend,
    onOpenSettings,
  } = $props();

  /** Filled in for every ticket without any column existing for them. */
  const BUILT_IN = [
    "serial",
    "serial_plain",
    "ticket_code",
    "event",
    "issued_at",
    "quantity",
  ];

  let newName = $state("");
  let creating = $state(null); // "new" | "copy" | null
  let quantity = $state(100);
  let combined = $state(false);
  let showHelp = $state(false);

  let fromBuyers = $derived(!!buyerFile);
  let ticketCount = $derived(fromBuyers ? buyerFile.import.tickets : Number(quantity) || 0);

  // Everything a template string can refer to here: what the design and copy
  // already mention, what the imported file offers, what the last run used,
  // and the ones that never need a column.
  let variables = $derived([
    ...new Set([
      ...BUILT_IN,
      ...(required ?? []),
      ...(buyerFile?.import.tokens ?? []),
      ...(event?.columns ?? []),
    ]),
  ]);

  // Columns the CSV has to carry. Anything missing is named rather than
  // counted, because a run that silently prints an empty seat number is only
  // discovered at the door.
  let missing = $derived(
    fromBuyers ? (required ?? []).filter((c) => !buyerFile.import.tokens.includes(c)) : [],
  );

  async function create() {
    const name = newName.trim();
    if (!name) return;
    if (creating === "copy") await onDuplicate(name);
    else await onNewEvent(name);
    newName = "";
    creating = null;
  }

  function startCopy() {
    // Pre-filled with the year rolled forward, which is what a duplicate is
    // almost always for.
    newName = (event?.name ?? "").replace(/\b(20\d{2})\b/, (y) => String(Number(y) + 1));
    creating = "copy";
  }

  /** Insert a variable at the caret, rather than making people type braces. */
  function insertVariable(token) {
    const el = document.getElementById("email-body");
    const at = el?.selectionStart ?? event.email.body.length;
    const body = event.email.body;
    event.email.body = body.slice(0, at) + `{{${token}}}` + body.slice(at);
  }
</script>

<label class="input-label" for="event">Event</label>
<div class="event-row">
  <select
    id="event"
    class="field event-select js-first-field"
    value={event?.id ?? ""}
    onchange={(e) => onPickEvent(e.currentTarget.value || null)}>
    <option value="">Choose an event</option>
    {#each events as ev}
      <option value={ev.id}>{ev.name} &middot; {ev.issued} issued</option>
    {/each}
  </select>
  {#if creating}
    <input
      class="field new-name"
      bind:value={newName}
      placeholder="Summer Fest 2027"
      onkeydown={(e) => e.key === "Enter" && create()} />
    <button class="text-button" onclick={create} disabled={!newName.trim()}>
      {creating === "copy" ? "Duplicate" : "Create"}
    </button>
    <button class="text-button" onclick={() => (creating = null)}>Cancel</button>
  {:else}
    <button class="text-button" onclick={() => (creating = "new")}>New event</button>
    {#if event}
      <button class="text-button" onclick={startCopy}>Duplicate</button>
    {/if}
  {/if}
</div>

{#if creating === "copy"}
  <p class="hint spaced">
    The design, the copy and the filename carry over. The duplicate gets its
    own signing key and starts at number 1, so last year's tickets cannot open
    this year's door.
  </p>
{/if}

{#if !event}
  <p class="hint spaced">
    An event keeps its own signing key, its ticket numbering and its record of
    who has already come in. Create one to begin.
  </p>
{:else}
  <div class="section first">
    <p class="section-caption">Template</p>
    <div class="rows">
      <div class="row">
        <div>
          <div class="row-label">{event.template?.name ?? "No template"}</div>
          <div class="row-helper">
            {#if event.template}
              {event.template.pages > 1
                ? `${event.template.pages} pages, all of them in every ticket.`
                : "Every ticket is a copy of this page."}
              Your design is laid over it; the file itself is never rewritten.
            {:else}
              Issue onto a blank A4 sheet, or choose your own design.
            {/if}
          </div>
        </div>
        <div class="swatch-row">
          <button class="text-button" onclick={onPickTemplate}>
            {event.template ? "Replace" : "Choose"}
          </button>
          {#if event.template}
            <button class="text-button" onclick={() => (event.template = null)}>Remove</button>
          {/if}
        </div>
      </div>

      {#if (event.template?.pages ?? 1) > 1}
        <div class="row">
          <div>
            <div class="row-label">Page to lay on</div>
            <div class="row-helper">The others are still included, untouched.</div>
          </div>
          <select
            class="field"
            value={event.template.page}
            onchange={(e) => (event.template.page = Number(e.currentTarget.value))}>
            {#each Array(event.template.pages) as _, i}
              <option value={i}>Page {i + 1}</option>
            {/each}
          </select>
        </div>
      {/if}
    </div>
  </div>

  <div class="section">
    <p class="section-caption">Design</p>
    <Layout bind:elements={event.layout} template={event.template} {variables} />
    {#if verdict && verdict.level !== "good"}
      <p class="hint verdict {verdict.level}">{verdict.notes[0]}</p>
    {/if}
  </div>

  <div class="section">
    <p class="section-caption">Who</p>
    <div class="rows">
      <div class="row">
        <div>
          <div class="row-label">Columns this event needs</div>
          <div class="row-helper">
            {#if required?.length}
              Taken from your design, your email copy and the filename. Add a
              column, use it in a text box, and it appears here.
            {:else}
              Nothing on this ticket refers to a column yet, so a plain
              numbered run is all it needs.
            {/if}
          </div>
        </div>
        <button class="text-button" onclick={onExampleCsv}>Example CSV</button>
      </div>

      {#if required?.length}
        <div class="row stack tight">
          <div class="vars">
            {#each required as column}
              <span
                class="chip"
                class:absent={missing.includes(column)}
                title={missing.includes(column) ? "Not in the imported file" : ""}>
                {column}
              </span>
            {/each}
          </div>
          {#if missing.length}
            <p class="hint warn">
              The imported file has no {missing.join(", ")} column. Those spots
              print as written rather than blank, so you can see what is
              missing.
            </p>
          {/if}
        </div>
      {/if}

      <div class="row">
        <div>
          <div class="row-label">{buyerFile ? buyerFile.name : "No buyer list"}</div>
          <div class="row-helper">
            {#if buyerFile}
              {buyerFile.import.rows.length} rows, {buyerFile.import.tickets} tickets.
              Every column comes through as a variable, whatever it is called.
            {:else}
              Import a CSV to get one file per person, or issue a plain
              numbered run.
            {/if}
          </div>
        </div>
        <div class="swatch-row">
          <button class="text-button" onclick={onPickBuyers}>
            {buyerFile ? "Replace" : "Import CSV"}
          </button>
          {#if buyerFile}
            <button class="text-button" onclick={onClearBuyers}>Remove</button>
          {/if}
        </div>
      </div>

      {#if buyerFile}
        <!-- Only these two columns mean anything to the app itself. Every
             other column is just a variable, so it needs no mapping. -->
        {#each [["Address to send to", "email"], ["Tickets per row", "quantity"]] as [label, field]}
          <div class="row">
            <div class="row-label">{label}</div>
            <select
              class="field"
              value={buyerFile.mapping[field] ?? ""}
              onchange={(e) =>
                onRemap({
                  ...buyerFile.mapping,
                  [field]: e.currentTarget.value === "" ? null : Number(e.currentTarget.value),
                })}>
              <option value="">Not used</option>
              {#each buyerFile.headers as h, i}
                <option value={i}>{h}</option>
              {/each}
            </select>
          </div>
        {/each}

        <div class="row stack tight">
          <div class="row-label">Variables from this file</div>
          <div class="vars">
            {#each buyerFile.import.tokens as token}
              <span class="chip">{token}</span>
            {/each}
          </div>
        </div>
      {:else}
        <div class="row">
          <div>
            <div class="row-label">How many</div>
            <div class="row-helper">Numbered from {event.nextSerial ?? 1}.</div>
          </div>
          <input class="field num-in" type="number" min="1" max="5000" bind:value={quantity} />
        </div>
      {/if}
    </div>

    {#if buyerFile?.import.problems.length}
      <!-- Listed in full rather than counted. A list that quietly drops eleven
           people is discovered at the door, not here. -->
      <ul class="problems">
        {#each buyerFile.import.problems as p}
          <li><span class="line">Line {p.line}</span><span>{p.reason}</span></li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="section">
    <p class="section-caption">Issue</p>
    <div class="rows">
      <div class="row">
        <div>
          <div class="row-label">Filename</div>
          <div class="row-helper">
            A template like everything else, so a run can be filed by surname
            or seat rather than only by number.
          </div>
        </div>
        <input class="field wide mono" bind:value={event.filename} spellcheck="false" />
      </div>

      <div class="row">
        <div>
          <div class="row-label">Also write one combined PDF</div>
          <div class="row-helper">
            Every ticket in one file, for printing a roll. Not what you email
            to anyone.
          </div>
        </div>
        <label class="toggle">
          <input type="checkbox" bind:checked={combined} />
          <span class="toggle-track"><span class="toggle-knob"></span></span>
        </label>
      </div>

      <div class="row">
        <div>
          <div class="row-label">Issue {ticketCount || ""} tickets</div>
          <div class="row-helper">
            Writes a PDF per ticket, a CSV manifest and the public key into a
            folder you choose.
          </div>
        </div>
        <button
          class="btn"
          disabled={busy || ticketCount === 0}
          onclick={() => onIssue({ quantity: Number(quantity) || 0, combined })}>
          <Icon name="folder" />
          Choose folder
        </button>
      </div>
    </div>
  </div>

  {#if outcome}
    <div class="section">
      <p class="section-caption">Issued</p>
      <div class="rows">
        <div class="row">
          <div>
            <div class="row-label">
              {outcome.count} tickets, numbers {outcome.firstSerial} to {outcome.lastSerial}
            </div>
            <div class="row-helper">{outcome.ticketsDir}</div>
          </div>
          <button class="text-button" onclick={onReveal}>
            <Icon name="folder" /> Show in folder
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if sendable && sendable.pending + sendable.sent + sendable.failed > 0}
    <div class="section">
      <p class="section-caption">Send</p>
      <div class="rows">
        <div class="row">
          <div>
            <div class="row-label">Subject</div>
            <div class="row-helper">Every variable below works here too.</div>
          </div>
          <input
            id="email-subject"
            class="field wide"
            bind:value={event.email.subject}
            spellcheck="false" />
        </div>

        <div class="row stack">
          <div class="row-label">Message</div>
          <textarea
            id="email-body"
            class="field body"
            bind:value={event.email.body}
            spellcheck="false"></textarea>
          <div class="vars">
            {#each variables as token}
              <button class="chip" onclick={() => insertVariable(token)}>{token}</button>
            {/each}
          </div>
        </div>

        <div class="row">
          <div>
            <div class="row-label">
              {sendable.pending} to send
              {#if sendable.sent}<span class="muted">&middot; {sendable.sent} sent</span>{/if}
              {#if sendable.failed}<span class="bad-text">&middot; {sendable.failed} failed</span>{/if}
              {#if sendable.noAddress}
                <span class="muted">&middot; {sendable.noAddress} without an address</span>
              {/if}
            </div>
            <div class="row-helper">
              {#if !sendReady}
                Sending needs a Resend API key and a verified from address.
              {:else}
                Each ticket is attached to its own message, and its number is
                written back into the manifest as it goes. A run that stops
                part way resumes by sending only what is still pending.
              {/if}
            </div>
          </div>
          {#if !sendReady}
            <button class="text-button" onclick={onOpenSettings}>Set up sending</button>
          {:else if sending}
            <button class="text-button" onclick={onCancelSend}>Stop</button>
          {:else}
            <button
              class="btn"
              disabled={busy || sendable.pending + sendable.failed === 0}
              onclick={() => onSend(sendable.failed > 0)}>
              <Icon name="download" />
              {sendable.failed > 0 && sendable.pending === 0 ? "Retry failed" : "Send"}
            </button>
          {/if}
        </div>

        {#if sending}
          <div class="row stack">
            <div class="row-label">
              {sending.done} of {sending.total}
              {#if sending.current}<span class="muted">&middot; {sending.current}</span>{/if}
            </div>
            <div class="progress">
              <span style="width:{(sending.done / Math.max(1, sending.total)) * 100}%"></span>
            </div>
          </div>
        {/if}
      </div>

      {#if sendable.failures?.length}
        <ul class="problems">
          {#each sendable.failures as f}
            <li><span class="line">#{f.serial}</span><span>{f.email}: {f.reason}</span></li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}

  <div class="section">
    <button class="text-button help-toggle" onclick={() => (showHelp = !showHelp)}>
      {showHelp ? "Hide" : "How checking works"}
    </button>
    {#if showHelp}
      <div class="help">
        <p>
          Every ticket carries a signature made with this event's private key,
          which never leaves this machine. The exported
          <code class="mono">public.key</code> is the other half: it can check a
          ticket but cannot create one, so it is safe to hand to whoever is on
          the door.
        </p>
        <p>
          The <strong>Check</strong> tab does this for you, and also refuses a
          ticket that has already been scanned. To use your own scanner instead,
          a payload looks like
          <code class="mono">TKT1.&lt;body&gt;.&lt;signature&gt;</code>, both
          parts base64url, where the body is
          <code class="mono">&lt;event&gt;|&lt;serial&gt;</code>. Verify the
          signature over the body bytes with Ed25519 before parsing anything out
          of it.
        </p>
        <p class="caveat">
          A signature proves a ticket is genuine. It cannot stop the same ticket
          being used twice, which needs a record kept where tickets are checked.
          The Check tab keeps that record on this machine, for one door.
        </p>
      </div>
    {/if}
  </div>
{/if}

<style>
  .event-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .event-select {
    width: 260px;
  }
  .new-name {
    width: 200px;
  }
  .spaced {
    margin-top: 12px;
    max-width: 440px;
  }
  .section.first {
    margin-top: 18px;
  }

  .verdict {
    margin-top: 8px;
  }
  .verdict.risky {
    color: var(--warn);
  }
  .verdict.bad {
    color: var(--bad);
  }

  .num-in {
    width: 96px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .wide {
    width: 260px;
  }
  .row.stack {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
  .row.tight {
    gap: 6px;
  }
  .body {
    min-height: 110px;
    font-size: 12.5px;
  }

  .vars {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-soft);
    background: var(--chip-bg);
    border: 1px solid var(--hairline);
    border-radius: 5px;
    padding: 2px 6px;
  }
  button.chip {
    cursor: pointer;
    transition: background 0.12s ease, color 0.12s ease;
  }
  button.chip:hover {
    background: var(--accent-soft);
    color: var(--accent);
  }
  /* Named rather than hidden: a column the design wants and the file lacks is
     the single most common reason a run comes out wrong. */
  .chip.absent {
    color: var(--warn);
    border-color: var(--warn);
    border-style: dashed;
    background: none;
  }
  .warn {
    color: var(--warn);
  }

  .problems {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 7px;
    max-height: 200px;
    overflow-y: auto;
    margin-top: 8px;
  }
  .problems li {
    display: flex;
    gap: 10px;
    font-size: 12px;
    line-height: 1.4;
  }
  .line {
    color: var(--bad);
    flex: 0 0 56px;
    font-variant-numeric: tabular-nums;
  }

  .muted {
    color: var(--text-faint);
    font-weight: 400;
  }
  .bad-text {
    color: var(--bad);
    font-weight: 400;
  }
  /* A plain bar with no background track: the app has no other progress
     indicator, and a filled track would read as a dashboard widget. */
  .progress {
    height: 3px;
    border-radius: 2px;
    background: var(--track);
    overflow: hidden;
  }
  .progress span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.2s ease;
  }

  .help-toggle {
    margin-left: -8px;
  }
  .help {
    margin-top: 8px;
    max-width: 560px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--text-soft);
  }
  .help code {
    background: var(--chip-bg);
    border-radius: 4px;
    padding: 1px 5px;
  }
  .caveat {
    color: var(--text-faint);
    border-left: 2px solid var(--hairline-strong);
    padding-left: 10px;
  }
</style>
