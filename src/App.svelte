<script>
  import * as api from "./lib/api.js";
  import Icon from "./lib/Icon.svelte";
  import Preview from "./lib/Preview.svelte";
  import Link from "./lib/modes/Link.svelte";
  import Wifi from "./lib/modes/Wifi.svelte";
  import Contact from "./lib/modes/Contact.svelte";
  import Batch from "./lib/modes/Batch.svelte";
  import Tickets from "./lib/modes/Tickets.svelte";
  import Decode from "./lib/modes/Decode.svelte";
  import Check from "./lib/modes/Check.svelte";
  import Settings from "./lib/modes/Settings.svelte";

  // Long enough that a fast typist does not queue a render per keystroke,
  // short enough that the preview still feels attached to the keyboard.
  const DEBOUNCE_MS = 120;

  const GROUPS = [
    {
      caption: "Make",
      modes: [
        { id: "link", label: "Link", icon: "link", hint: "Paste a link to begin." },
        { id: "wifi", label: "Wi-Fi", icon: "wifi", hint: "Enter a network name to begin." },
        { id: "contact", label: "Contact", icon: "user", hint: "Fill in a name to begin." },
        { id: "batch", label: "Batch", icon: "list", hint: "Paste one entry per line." },
      ],
    },
    {
      caption: "Events",
      modes: [
        { id: "tickets", label: "Tickets", icon: "ticket", hint: "Choose an event to begin." },
        { id: "check", label: "Check", icon: "scan", hint: "" },
      ],
    },
    {
      caption: "Tools",
      modes: [
        { id: "decode", label: "Read a code", icon: "scan", hint: "Choose an image to read." },
        { id: "settings", label: "Settings", icon: "gear", hint: "" },
      ],
    },
  ];
  const MODES = GROUPS.flatMap((g) => g.modes);

  let mode = $state("link");

  let link = $state({ input: "", kind: "link" });
  // Once the user picks a type by hand, detection stops overriding them.
  let kindTouched = $state(false);
  let wifi = $state({ ssid: "", password: "", security: "wpa", hidden: false });
  let batch = $state({ text: "", kind: "link", naming: "content" });
  let batchOutcome = $state(null);
  let eventList = $state([]);
  let event = $state(null);
  let buyerFile = $state(null);
  // Which columns the current design and copy refer to. Computed in Rust, by
  // the same scanner that renders them, so the list cannot drift from what an
  // issue run actually looks up.
  let required = $state([]);
  let ticketOutcome = $state(null);
  let doorEventId = $state(null);
  let doorInfo = $state(null);
  let doorRecent = $state([]);
  let settings = $state({ apiKey: "", from: "", replyTo: "" });
  let eventTickets = $state([]);
  let sending = $state(null);
  let lastReport = $state(null);
  let decode = $state({ fileName: "", results: [], error: "" });
  // Session only, deliberately. A file on disk would mean corruption handling
  // and a settings toggle for something whose whole value is being one click
  // away while you work.
  let recent = $state([]);
  let contact = $state({
    firstName: "",
    lastName: "",
    org: "",
    title: "",
    phone: "",
    email: "",
    url: "",
  });

  let ecc = $state("m");
  let logo = $state(null);
  let logoFraction = $state(0.2);
  let scale = $state(8);
  let quietZone = $state(4);
  let dark = $state("#000000");
  let light = $state("#ffffff");
  let format = $state("svg");

  let result = $state(null);
  let error = $state("");
  let busy = $state(false);
  let lastPath = $state("");
  let toast = $state("");

  let toastTimer;
  // Guards against an earlier render resolving after a later one and
  // overwriting the newer preview.
  let seq = 0;

  // In batch mode the preview shows the first entry, so the user can see the
  // shape of what a thousand files are about to look like before writing them.
  let firstBatchLine = $derived(
    batch.text
      .split("\n")
      .map((l) => l.trim())
      .find((l) => l) ?? "",
  );

  // A stand-in of the right length and character set, so the preview shows a
  // code of realistically the same density as the real signed ones without
  // touching the event key.
  let sampleTicket = $derived(
    ticketOutcome?.sample ??
      "TKT1." + "A".repeat(20) + "." + "B".repeat(86),
  );

  let payload = $derived(
    mode === "tickets"
      ? { kind: "text", input: sampleTicket }
      : mode === "batch"
      ? { kind: batch.kind, input: firstBatchLine }
      : mode === "wifi"
      ? { kind: "wifi", ...wifi }
      : mode === "contact"
        ? { kind: "contact", ...contact }
        : { kind: link.kind, input: link.input },
  );

  let hasContent = $derived(
    mode === "decode" || mode === "check" || mode === "settings"
      ? false
      : mode === "tickets"
      ? !!event
      : mode === "batch"
      ? firstBatchLine !== ""
      : mode === "wifi"
      ? wifi.ssid.trim() !== ""
      : mode === "contact"
        ? Object.values(contact).some((v) => v.trim() !== "")
        : link.input.trim() !== "",
  );

  // A logo destroys modules, and only the highest error correction level
  // recovers reliably. The select shows the forced value rather than lying
  // about what is being used.
  // On the tickets tab these controls belong to the event, which is what an
  // issue run actually reads. Driving the loose ones there would show a
  // preview in colours the printed tickets never use.
  let onEvent = $derived(mode === "tickets" && !!event);
  let eccValue = $derived(onEvent ? event.ecc : ecc);
  let darkValue = $derived(onEvent ? event.dark : dark);
  let lightValue = $derived(onEvent ? event.light : light);

  function setEcc(v) {
    if (onEvent) event.ecc = v;
    else ecc = v;
  }
  function setColour(which, v) {
    if (onEvent) event[which] = v;
    else if (which === "dark") dark = v;
    else light = v;
  }

  let effectiveEcc = $derived(logo ? "h" : eccValue);
  let req = $derived({
    payload,
    ecc: effectiveEcc,
    scale,
    quietZone,
    dark: darkValue,
    light: lightValue,
    logoFraction: logo ? logoFraction : null,
  });
  let sendReady = $derived(!!settings.apiKey?.trim() && !!settings.from?.trim());
  let sendable = $derived.by(() => {
    const counts = { pending: 0, sent: 0, failed: 0, noAddress: 0 };
    for (const t of eventTickets) {
      const status = t.delivery?.status ?? "pending";
      if (status === "pending") counts.pending++;
      else if (status === "sent") counts.sent++;
      else if (status === "failed") counts.failed++;
      else counts.noAddress++;
    }
    return { ...counts, failures: lastReport?.failures ?? [] };
  });

  let isDefaultColours = $derived(darkValue === "#000000" && lightValue === "#ffffff");
  let emptyHint = $derived(MODES.find((m) => m.id === mode)?.hint ?? "");

  $effect(() => {
    const r = req;
    if (!hasContent) {
      result = null;
      error = "";
      return;
    }
    const t = setTimeout(() => render(r), DEBOUNCE_MS);
    return () => clearTimeout(t);
  });

  async function render(r) {
    const mine = ++seq;
    try {
      if (r.payload.kind !== "wifi" && r.payload.kind !== "contact" && !kindTouched) {
        const detected = await api.detectKind(r.payload.input);
        if (mine !== seq) return;
        // Changing kind re-runs the effect, so let that render happen rather
        // than doing the work twice here.
        if (detected !== link.kind) {
          link.kind = detected;
          return;
        }
      }
      const p = await api.preview(r);
      if (mine !== seq) return;
      result = p;
      error = "";
    } catch (e) {
      if (mine !== seq) return;
      result = null;
      error = String(e);
    }
  }

  function flash(msg) {
    toast = msg;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = ""), 2200);
  }

  function pickKind(k) {
    link.kind = k;
    kindTouched = true;
  }

  function focusFirstField() {
    const el = document.querySelector(".js-first-field");
    el?.focus();
    el?.select?.();
  }

  function setMode(id) {
    if (mode === id) return;
    mode = id;
    lastPath = "";
    // The new mode's first field is only in the DOM after this render.
    queueMicrotask(focusFirstField);
  }

  async function doSave() {
    if (!result || busy) return;
    busy = true;
    try {
      const path = await api.saveAs(req, format);
      if (path) {
        lastPath = path;
        remember({ encoded: result.encoded, kind: payload.kind, mode });
        flash(`Saved ${path.split(/[\\/]/).pop()}`);
      }
    } catch (e) {
      flash(String(e));
    } finally {
      busy = false;
    }
  }

  async function doCopyImage() {
    if (!result || busy) return;
    busy = true;
    try {
      await api.copyImage(req);
      remember({ encoded: result.encoded, kind: payload.kind, mode });
      flash("Image copied");
    } catch (e) {
      flash(String(e));
    } finally {
      busy = false;
    }
  }

  async function doCopyText() {
    if (!result) return;
    try {
      await api.copyText(result.encoded);
      flash("Contents copied");
    } catch (e) {
      flash(String(e));
    }
  }

  async function doBatchExport() {
    if (busy) return;
    busy = true;
    try {
      const outcome = await api.batchExport(req, {
        text: batch.text,
        format,
        naming: batch.naming,
      });
      if (outcome) {
        batchOutcome = outcome;
        flash(
          outcome.failures.length
            ? `${outcome.written} written, ${outcome.failures.length} failed`
            : `${outcome.written} files written`,
        );
      }
    } catch (e) {
      flash(String(e));
    } finally {
      busy = false;
    }
  }

  async function refreshTickets() {
    if (!event) {
      eventTickets = [];
      return;
    }
    try {
      eventTickets = await api.eventTickets(event.id);
    } catch {
      eventTickets = [];
    }
  }

  async function doSend(retryFailed) {
    if (!event || sending) return;
    lastReport = null;
    try {
      const total = await api.sendTickets(event.id, retryFailed);
      sending = { done: 0, total, current: "" };
    } catch (e) {
      flash(String(e));
    }
  }

  async function testSend(to) {
    try {
      await api.sendTest(to);
      return { ok: true };
    } catch (e) {
      return { ok: false, error: String(e) };
    }
  }

  async function refreshEvents() {
    try {
      eventList = await api.listEvents();
    } catch (e) {
      flash(String(e));
    }
  }

  async function refreshRequired() {
    if (!event) {
      required = [];
      return;
    }
    try {
      required = await api.requiredColumns(event.id);
    } catch {
      // The panel is guidance, not a gate. Failing to refresh it should not
      // interrupt whatever the user was doing.
    }
  }

  async function pickEvent(id) {
    buyerFile = null;
    ticketOutcome = null;
    if (!id) {
      event = null;
      required = [];
      return;
    }
    try {
      event = await api.loadEvent(id);
      await refreshTickets();
      await refreshRequired();
    } catch (e) {
      event = null;
      flash(String(e));
    }
  }

  async function newEvent(name) {
    try {
      event = await api.createEvent(name);
      ticketOutcome = null;
      buyerFile = null;
      await refreshEvents();
      await refreshRequired();
    } catch (e) {
      flash(String(e));
    }
  }

  /**
   * Copy the current event for another run.
   *
   * The buyer list is deliberately not carried over: it is last year's people,
   * and issuing to them again is exactly the mistake this makes easy.
   */
  async function duplicateEvent(name) {
    if (!event) return;
    try {
      await api.saveEvent(event);
      event = await api.duplicateEvent(event.id, name);
      ticketOutcome = null;
      buyerFile = null;
      await refreshEvents();
      await refreshTickets();
      await refreshRequired();
      flash(`Duplicated as ${event.name}`);
    } catch (e) {
      flash(String(e));
    }
  }

  async function exampleCsv() {
    if (!event) return;
    try {
      const path = await api.writeExampleCsv(event.id, `${event.id}-example.csv`);
      if (path) flash(`Saved ${path.split(/[\\/]/).pop()}`);
    } catch (e) {
      flash(String(e));
    }
  }

  async function chooseTemplate() {
    try {
      const info = await api.pickTemplate();
      if (info && event) {
        event.template = { path: info.path, name: info.name, page: 0, pages: info.pages };
      }
    } catch (e) {
      // Rejected templates (rotated pages, unreadable files) surface here,
      // which is why validation runs at pick time rather than at issue time.
      flash(String(e));
    }
  }

  async function chooseBuyers() {
    try {
      const file = await api.pickBuyers();
      if (file) buyerFile = file;
    } catch (e) {
      flash(String(e));
    }
  }

  async function remapBuyers(mapping) {
    if (!buyerFile) return;
    try {
      const parsed = await api.remapBuyers(buyerFile.path, mapping);
      buyerFile = { ...buyerFile, mapping, import: parsed };
    } catch (e) {
      flash(String(e));
    }
  }

  async function doIssue({ quantity, combined }) {
    if (!event || busy) return;
    busy = true;
    try {
      // Saved first, so the run uses exactly what is on screen rather than
      // whatever the debounced save last managed to write.
      await api.saveEvent(event);
      const outcome = await api.issue({
        eventId: event.id,
        rows: buyerFile ? buyerFile.import.rows : [],
        // Tokens rather than the original headers, because those are the keys
        // a ticket's fields are stored under and the manifest looks up.
        columns: buyerFile ? buyerFile.import.tokens : [],
        quantity,
        combined,
      });
      if (outcome) {
        ticketOutcome = outcome;
        event = await api.loadEvent(event.id);
        await refreshTickets();
        await refreshEvents();
        flash(`${outcome.count} tickets issued`);
      }
    } catch (e) {
      flash(String(e));
    } finally {
      busy = false;
    }
  }

  async function pickDoorEvent(id) {
    doorEventId = id;
    doorInfo = null;
    doorRecent = [];
    if (!id) return;
    try {
      doorInfo = await api.doorState(id);
    } catch (e) {
      flash(String(e));
    }
  }

  async function scan(scanned) {
    if (!doorEventId) return null;
    busy = true;
    try {
      const outcome = await api.checkTicket(doorEventId, scanned);
      doorRecent = [
        { ...outcome, at: new Date().toISOString() },
        ...doorRecent,
      ].slice(0, 12);
      doorInfo = await api.doorState(doorEventId);
      return outcome;
    } catch (e) {
      flash(String(e));
      return { outcome: "unreadable", reason: String(e) };
    } finally {
      busy = false;
    }
  }

  async function exportDoorLog() {
    if (!doorEventId) return;
    try {
      const path = await api.exportRedemptions(doorEventId, `${doorEventId}-admissions.csv`);
      if (path) flash(`Saved ${path.split(/[\\/]/).pop()}`);
    } catch (e) {
      flash(String(e));
    }
  }

  function resetColours() {
    setColour("dark", "#000000");
    setColour("light", "#ffffff");
  }

  async function chooseLogo() {
    try {
      const info = await api.pickLogo();
      if (info) logo = info;
    } catch (e) {
      flash(String(e));
    }
  }

  async function removeLogo() {
    logo = null;
    try {
      await api.clearLogo();
    } catch {
      // Nothing useful to tell the user: the logo is already gone from the
      // request, so the next render is correct either way.
    }
  }

  function onKeydown(e) {
    if (!(e.ctrlKey || e.metaKey)) return;
    const k = e.key.toLowerCase();
    if (k === "s") {
      e.preventDefault();
      doSave();
    } else if (k === "l") {
      e.preventDefault();
      focusFirstField();
      // Ctrl+Shift+C rather than Ctrl+C, which has to keep working as a plain
      // text copy while the caret is in a field.
    } else if (k === "c" && e.shiftKey) {
      e.preventDefault();
      doCopyImage();
    }
  }

  let settingsTimer;
  $effect(() => {
    const snapshot = JSON.stringify(settings);
    if (snapshot === '{"apiKey":"","from":"","replyTo":""}') return;
    clearTimeout(settingsTimer);
    settingsTimer = setTimeout(() => {
      api.saveSettings(JSON.parse(snapshot)).catch((e) => flash(String(e)));
    }, 400);
  });

  let saveTimer;
  $effect(() => {
    // Reading the whole record is what subscribes this to every field of it.
    const snapshot = event ? JSON.stringify(event) : null;
    if (!snapshot) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      try {
        await api.saveEvent(JSON.parse(snapshot));
        eventList = await api.listEvents();
        // Only after the save, or the scanner would read the previous design.
        await refreshRequired();
      } catch (e) {
        flash(String(e));
      }
    }, 400);
  });

  $effect(() => {
    refreshEvents();
    api.loadSettings().then((c) => (settings = c)).catch(() => {});

    const progress = api.onSendProgress((p) => (sending = p));
    const done = api.onSendDone((report) => {
      sending = null;
      lastReport = report;
      refreshTickets();
      flash(
        report.cancelled
          ? `Stopped after ${report.sent} sent`
          : report.failed
            ? `${report.sent} sent, ${report.failed} failed`
            : `${report.sent} tickets sent`,
      );
    });
    return () => {
      progress.then((f) => f()).catch(() => {});
      done.then((f) => f()).catch(() => {});
    };
  });

  $effect(() => {
    focusFirstField();
  });

  $effect(() => {
    const stop = api.onClipboardCode((text) => {
      const value = (text ?? "").trim();
      if (!value) {
        flash("Clipboard is empty");
        return;
      }
      mode = "link";
      // Cleared so detection runs again on whatever was just pasted, rather
      // than inheriting a type the user chose for something unrelated.
      kindTouched = false;
      link = { input: value, kind: link.kind };
      queueMicrotask(focusFirstField);
    });
    return () => {
      stop.then((fn) => fn()).catch(() => {});
    };
  });
</script>

<svelte:window on:keydown={onKeydown} />

<main class="shell">
  <nav class="rail" aria-label="Sections">
    {#each GROUPS as group}
      <p class="rail-caption">{group.caption}</p>
      {#each group.modes as m}
        <button
          class="nav-item"
          aria-current={mode === m.id ? "page" : undefined}
          onclick={() => setMode(m.id)}>
          <Icon name={m.icon} />
          {m.label}
        </button>
      {/each}
    {/each}
  </nav>

  <section class="pane scroll">
    {#if mode === "link"}
      <Link bind:data={link} encoded={result?.encoded ?? ""} onKindPick={pickKind} />

      {#if recent.length}
        <!-- Only what has actually been saved or copied, so this stays a list
             of finished work rather than a log of every keystroke. -->
        <div class="section">
          <p class="section-caption">Recent</p>
          <div class="rows">
            {#each recent as entry}
              <button class="recent-row" onclick={() => restore(entry)}>
                <span class="recent-text mono">{entry.encoded}</span>
                <span class="recent-kind">{entry.mode}</span>
              </button>
            {/each}
          </div>
        </div>
      {/if}
    {:else if mode === "wifi"}
      <Wifi bind:data={wifi} />
    {:else if mode === "contact"}
      <Contact bind:data={contact} />
    {:else if mode === "batch"}
      <Batch
        bind:data={batch}
        {format}
        {busy}
        outcome={batchOutcome}
        onExport={doBatchExport}
        onReveal={() => api.reveal(batchOutcome.directory)} />
    {:else if mode === "tickets"}
      <Tickets
        events={eventList}
        bind:event
        {buyerFile}
        {required}
        {busy}
        verdict={result?.verdict ?? null}
        outcome={ticketOutcome}
        onPickEvent={pickEvent}
        onNewEvent={newEvent}
        onDuplicate={duplicateEvent}
        onPickTemplate={chooseTemplate}
        onPickBuyers={chooseBuyers}
        onRemap={remapBuyers}
        onClearBuyers={() => (buyerFile = null)}
        onExampleCsv={exampleCsv}
        onIssue={doIssue}
        onReveal={() => api.reveal(ticketOutcome.ticketsDir)}
        {sendable}
        {sendReady}
        {sending}
        onSend={doSend}
        onCancelSend={() => api.cancelSend()}
        onOpenSettings={() => setMode("settings")} />
    {:else if mode === "check"}
      <Check
        events={eventList}
        eventId={doorEventId}
        door={doorInfo}
        recent={doorRecent}
        {busy}
        onPickEvent={pickDoorEvent}
        onScan={scan}
        onExport={exportDoorLog} />
    {:else if mode === "settings"}
      <Settings bind:config={settings} {busy} onTest={testSend} />
    {:else if mode === "decode"}
      <Decode bind:data={decode} {busy} onPick={doDecode} onCopy={copyFound} />
    {/if}

    {#if mode !== "decode" && mode !== "check" && mode !== "settings"}
    <div class="section">
      <p class="section-caption">Code</p>
      <div class="rows">
        <div class="row">
          <div>
            <div class="row-label">Error correction</div>
            <div class="row-helper">
              {#if logo}
                Held at High because a logo covers part of the code. Anything
                lower cannot recover the hidden modules.
              {:else}
                Higher levels stay readable when the code is scuffed or partly
                covered, at the cost of a denser pattern.
              {/if}
            </div>
          </div>
          <select
            class="field"
            value={eccValue}
            onchange={(e) => setEcc(e.currentTarget.value)}
            disabled={!!logo}>
            <option value="l">Low, 7%</option>
            <option value="m">Medium, 15%</option>
            <option value="q">Quartile, 25%</option>
            <option value="h">High, 30%</option>
          </select>
        </div>

        {#if mode !== "tickets"}
        <div class="row">
          <div>
            <div class="row-label">Size</div>
            <div class="row-helper">Affects the PNG only. The SVG scales to any size.</div>
          </div>
          <div class="slider">
            <input
              type="range"
              min="2"
              max="40"
              bind:value={scale}
              style="--fill: {((scale - 2) / 38) * 100}%"
              aria-label="Pixels per module" />
            <span class="slider-value">{result ? `${result.sizePx} px` : `${scale}x`}</span>
          </div>
        </div>

        <div class="row">
          <div>
            <div class="row-label">Quiet zone</div>
            <div class="row-helper">
              The margin scanners need to find the code. Four is the spec
              minimum; less than that starts failing on busy backgrounds.
            </div>
          </div>
          <div class="slider">
            <input
              type="range"
              min="0"
              max="16"
              bind:value={quietZone}
              style="--fill: {(quietZone / 16) * 100}%"
              aria-label="Quiet zone in modules" />
            <span class="slider-value">{quietZone}</span>
          </div>
        </div>
        {/if}
      </div>
    </div>

    {#if mode !== "tickets"}

    <div class="section">
      <p class="section-caption">Logo</p>
      <div class="rows">
        <div class="row">
          <div>
            <div class="row-label">Centre image</div>
            <div class="row-helper">
              {#if logo}
                {logo.name}, {logo.width} &times; {logo.height}
              {:else}
                PNG, JPEG, GIF or WebP. The code is checked after the logo goes
                on, so you find out here if it covers too much.
              {/if}
            </div>
          </div>
          <div class="swatch-row">
            <button class="text-button" onclick={chooseLogo}>
              {logo ? "Replace" : "Choose"}
            </button>
            {#if logo}
              <button class="text-button" onclick={removeLogo}>Remove</button>
            {/if}
          </div>
        </div>

        {#if logo}
          <div class="row">
            <div>
              <div class="row-label">Logo size</div>
              <div class="row-helper">Share of the code's width.</div>
            </div>
            <div class="slider">
              <input
                type="range"
                min="5"
                max="35"
                value={Math.round(logoFraction * 100)}
                oninput={(e) => (logoFraction = Number(e.currentTarget.value) / 100)}
                style="--fill: {((logoFraction * 100 - 5) / 30) * 100}%"
                aria-label="Logo size as a percentage of the code" />
              <span class="slider-value">{Math.round(logoFraction * 100)}%</span>
            </div>
          </div>
        {/if}
      </div>
    </div>
    {/if}

    <div class="section">
      <p class="section-caption">Colour</p>
      <div class="rows">
        <div class="row">
          <div>
            <div class="row-label">Foreground and background</div>
            <div class="row-helper">Anything low contrast is flagged before you save it.</div>
          </div>
          <div class="swatch-row">
            <input
              type="color"
              value={darkValue}
              oninput={(e) => setColour("dark", e.currentTarget.value)}
              aria-label="Foreground colour" />
            <input
              type="color"
              value={lightValue}
              oninput={(e) => setColour("light", e.currentTarget.value)}
              aria-label="Background colour" />
            <button class="text-button" onclick={resetColours} disabled={isDefaultColours}>
              Reset
            </button>
          </div>
        </div>
      </div>
    </div>
    {/if}
  </section>

  <!-- The tickets tab carries its own preview in the positioning stage, and a
       third column does not fit beside it at a sane window width. -->
  {#if mode !== "decode" && mode !== "tickets" && mode !== "check" && mode !== "settings"}
  <Preview
    {result}
    {error}
    {busy}
    {format}
    {lastPath}
    {emptyHint}
    showExport={mode !== "batch" && mode !== "tickets"}
    caption={mode === "batch" ? "First entry" : mode === "tickets" ? "Sample ticket" : ""}
    onFormat={(f) => (format = f)}
    onSave={doSave}
    onCopyImage={doCopyImage}
    onCopyText={doCopyText}
    onReveal={() => api.reveal(lastPath)} />
  {/if}
</main>

{#if toast}
  <div class="toast" role="status">{toast}</div>
{/if}

<style>
  .shell {
    display: flex;
    height: 100vh;
  }

  .rail-caption {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-faint);
    padding: 0 10px;
    margin: 12px 0 4px;
  }
  .rail-caption:first-child {
    margin-top: 0;
  }

  .rail {
    flex: 0 0 168px;
    background: var(--bg-panel);
    border-right: 1px solid var(--hairline);
    padding: 14px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    user-select: none;
  }

  .pane {
    flex: 1;
    padding: 18px 24px 40px;
    min-width: 0;
  }

  .recent-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    width: 100%;
    padding: 9px 0;
    background: none;
    border: none;
    font-family: inherit;
    text-align: left;
    color: var(--text);
    cursor: pointer;
    border-radius: 4px;
    transition: background 0.12s ease;
  }
  .recent-row:hover {
    background: var(--hover);
  }
  .recent-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .recent-kind {
    font-size: 11px;
    color: var(--text-faint);
    flex: 0 0 auto;
  }

  .toast {
    position: fixed;
    left: 50%;
    bottom: 20px;
    transform: translateX(-50%);
    background: var(--control-bg);
    border: 1px solid var(--hairline-strong);
    border-radius: 999px;
    padding: 7px 16px;
    font-size: 12px;
    box-shadow: 0 8px 24px rgba(16, 16, 22, 0.24);
    pointer-events: none;
  }
</style>
