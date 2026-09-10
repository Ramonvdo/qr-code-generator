<script>
  import Icon from "./Icon.svelte";
  import * as api from "./api.js";

  let { elements = $bindable(), template, variables } = $props();

  /**
   * pdf.js is over a megabyte, and most sessions never open this tab. Loading
   * it on first use rather than at startup keeps the initial bundle small,
   * which is most of the point of a local app.
   */
  const loadRenderer = () => import("./pdfpreview.js");

  const CANVAS_W = 360;
  const A4_RATIO = 842 / 595;
  const A4_HEIGHT_PT = 842;

  // How far an arrow key moves a handle, as a fraction of the page. Dragging
  // gets you close; this is what gets you exact.
  const NUDGE = 0.002;
  const NUDGE_COARSE = 0.02;

  const FONTS = [
    ["helvetica", "Helvetica"],
    ["helveticaBold", "Helvetica Bold"],
    ["courier", "Courier"],
  ];
  const ALIGN = [
    ["left", "Left"],
    ["centre", "Centre"],
    ["right", "Right"],
  ];
  const KINDS = [
    ["text", "Text"],
    ["shape", "Box or line"],
    ["image", "Image"],
  ];

  let canvasEl = $state(null);
  let stageEl = $state(null);
  let stageHeight = $state(0);
  let pageRatio = $state(A4_RATIO);
  let pageHeightPt = $state(A4_HEIGHT_PT);
  let renderError = $state("");
  let dragging = $state(null);
  let selectedId = $state(null);

  let selected = $derived(elements.find((e) => e.id === selectedId) ?? null);
  let hasQr = $derived(elements.some((e) => e.kind === "qr"));

  // One point on the page, in screen pixels. Text is specified in points, so
  // without this the preview shows a size that has nothing to do with print.
  let pxPerPoint = $derived(stageHeight > 0 ? stageHeight / pageHeightPt : 0.6);

  $effect(() => {
    const path = template?.path ?? null;
    const page = template?.page ?? 0;
    if (!canvasEl) return;
    renderError = "";

    if (!path) {
      pageRatio = A4_RATIO;
      pageHeightPt = A4_HEIGHT_PT;
      // Only the backing resolution is set here. CSS decides the displayed
      // size, so the stage can be fluid without the drawing going out of step.
      canvasEl.width = CANVAS_W;
      canvasEl.height = Math.round(CANVAS_W * A4_RATIO);
      const ctx = canvasEl.getContext("2d");
      ctx.fillStyle = "#ffffff";
      ctx.fillRect(0, 0, canvasEl.width, canvasEl.height);
      return;
    }

    loadRenderer()
      .then((m) => m.renderTemplatePage(path, canvasEl, CANVAS_W, page + 1))
      .then((p) => {
        pageRatio = p.ratio;
        pageHeightPt = p.height;
      })
      .catch((e) => (renderError = String(e)));
  });

  function fractionAt(e) {
    const box = stageEl.getBoundingClientRect();
    return {
      x: Math.min(1, Math.max(0, (e.clientX - box.left) / box.width)),
      y: Math.min(1, Math.max(0, (e.clientY - box.top) / box.height)),
    };
  }

  const clamp = (v) => Math.min(1, Math.max(0, v));

  function moveTo(element, x, y) {
    element.x = clamp(x);
    element.y = clamp(y);
  }

  function startDrag(element, e) {
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    selectedId = element.id;
    const at = fractionAt(e);
    // Remember where inside the handle the grab happened, so it does not jump
    // its own corner under the cursor on the first move.
    dragging = { id: element.id, dx: at.x - element.x, dy: at.y - element.y };
  }

  function onDrag(e) {
    if (!dragging) return;
    const element = elements.find((el) => el.id === dragging.id);
    if (!element) return;
    const at = fractionAt(e);
    moveTo(element, at.x - dragging.dx, at.y - dragging.dy);
  }

  function endDrag(e) {
    if (dragging) e.currentTarget.releasePointerCapture?.(e.pointerId);
    dragging = null;
  }

  /** Arrow keys nudge the focused element, which is how you hit an exact spot. */
  function onHandleKey(element, e) {
    const step = e.shiftKey ? NUDGE_COARSE : NUDGE;
    const moves = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const move = moves[e.key];
    if (!move) return;
    e.preventDefault();
    selectedId = element.id;
    moveTo(element, element.x + move[0], element.y + move[1]);
  }

  function add(kind) {
    const element = api.newElement(kind);
    elements = [...elements, element];
    selectedId = element.id;
  }

  function duplicate() {
    if (!selected) return;
    const copy = {
      ...$state.snapshot(selected),
      id: `${selected.kind}-${Math.random().toString(36).slice(2, 8)}`,
      // Offset a little, or the copy hides exactly behind the original and
      // looks as though nothing happened.
      x: clamp(selected.x + 0.02),
      y: clamp(selected.y + 0.02),
    };
    elements = [...elements, copy];
    selectedId = copy.id;
  }

  function remove() {
    if (!selected) return;
    elements = elements.filter((e) => e.id !== selectedId);
    selectedId = null;
  }

  /** Move the selected element through the stacking order. */
  function reorder(by) {
    const i = elements.findIndex((e) => e.id === selectedId);
    const j = i + by;
    if (i < 0 || j < 0 || j >= elements.length) return;
    const next = [...elements];
    [next[i], next[j]] = [next[j], next[i]];
    elements = next;
  }

  /** Insert a variable at the caret, rather than making people type braces. */
  function insertVariable(token) {
    if (!selected) return;
    if (selected.kind === "image") {
      selected.column = token;
      return;
    }
    if (selected.kind !== "text") return;
    const field = document.getElementById("element-template");
    const at = field?.selectionStart ?? selected.template.length;
    selected.template =
      selected.template.slice(0, at) + `{{${token}}}` + selected.template.slice(at);
  }

  const pct = (n) => `${(n * 100).toFixed(2)}%`;

  const ALIGN_SHIFT = { left: "0", centre: "-50%", right: "-100%" };

  /**
   * Where a handle sits, and which way it turns.
   *
   * The PDF draws every element from an origin and extends upwards from it, so
   * that origin is the box's bottom-left for a code, an image or a box, and the
   * baseline start for text. A code stores its top-left, text stores its
   * baseline, which is why one is positioned from the top and the other from
   * the bottom. Rotation flips sign: PDF turns anticlockwise about the origin,
   * CSS turns clockwise.
   */
  function handleStyle(el) {
    const parts = [`left:${pct(el.x)}`, `opacity:${Math.max(0.25, el.opacity)}`];
    const turn = `rotate(${-el.rotation}deg)`;

    if (el.kind === "text") {
      parts.push(
        `bottom:calc(100% - ${pct(el.y)})`,
        `font-size:${Math.max(6, el.points * pxPerPoint).toFixed(2)}px`,
        `color:${el.colour}`,
        // Aligning happens inside the rotated frame in the PDF, so the shift
        // has to come after the turn here too.
        `transform:${turn} translateX(${ALIGN_SHIFT[el.align] ?? "0"})`,
      );
      return parts.join(";");
    }

    parts.push(`top:${pct(el.y)}`, `transform:${turn}`);
    if (el.kind === "qr") parts.push(`width:${pct(el.size)}`);
    else parts.push(`width:${pct(el.width)}`, `height:${pct(el.height)}`);
    if (el.kind === "shape" && el.fill) parts.push(`background:${el.fill}`);
    if (el.kind === "shape" && el.stroke) parts.push(`border-color:${el.stroke}`);
    return parts.join(";");
  }

  function label(element) {
    if (element.kind === "qr") return "Code";
    if (element.kind === "text") return element.template || "Empty text";
    if (element.kind === "image") return `Image from {{${element.column}}}`;
    return element.fill || element.stroke ? "Box" : "Empty box";
  }
</script>

<div class="layout">
  <div class="stage-wrap">
    <p class="stage-label">
      {template ? "Drag, or use arrow keys" : "Blank A4, drag to position"}
    </p>
    <div
      class="stage"
      bind:this={stageEl}
      bind:clientHeight={stageHeight}
      style="aspect-ratio: {1 / pageRatio}">
      <canvas bind:this={canvasEl}></canvas>

      <!-- Move and release live on the handles rather than the stage. Pointer
           capture routes every event to the grabbed element, so a drag keeps
           working past the edge of the page, and the only interactive
           elements stay real, focusable buttons. -->
      {#each elements as element (element.id)}
        <button
          class="handle {element.kind}"
          class:on={selectedId === element.id}
          style={handleStyle(element)}
          onpointerdown={(e) => startDrag(element, e)}
          onpointermove={onDrag}
          onpointerup={endDrag}
          onpointercancel={endDrag}
          onkeydown={(e) => onHandleKey(element, e)}
          onfocus={() => (selectedId = element.id)}
          aria-label={label(element)}>
          {#if element.kind === "qr"}
            <span class="grid"></span>
          {:else if element.kind === "text"}
            <span class="preview-text">{element.template}</span>
          {:else if element.kind === "image"}
            <Icon name="folder" size={13} />
          {/if}
        </button>
      {/each}
    </div>
    {#if renderError}
      <p class="hint err">{renderError}</p>
    {/if}
    {#if !hasQr}
      <p class="hint warn">
        This design has no code on it. Add one, or the tickets cannot be
        scanned.
      </p>
    {/if}
  </div>

  <div class="panel">
    <p class="section-caption">On the ticket</p>
    <!-- The list is the stacking order, top of the list drawn first. -->
    <div class="rows">
      {#each elements as element (element.id)}
        <button
          class="item"
          class:on={selectedId === element.id}
          onclick={() => (selectedId = element.id)}>
          <Icon
            name={element.kind === "qr"
              ? "list"
              : element.kind === "text"
                ? "user"
                : element.kind === "image"
                  ? "folder"
                  : "stop"}
            size={14} />
          <span class="item-label mono">{label(element)}</span>
        </button>
      {/each}
    </div>

    <div class="add-row">
      {#each KINDS as [kind, name]}
        <button class="text-button" onclick={() => add(kind)}>+ {name}</button>
      {/each}
      {#if !hasQr}
        <button class="text-button" onclick={() => add("qr")}>+ Code</button>
      {/if}
    </div>

    {#if selected}
      <div class="section">
        <p class="section-caption">Selected</p>
        <div class="rows">
          {#if selected.kind === "text"}
            <div class="row stack">
              <div class="row-label">Text</div>
              <textarea
                id="element-template"
                class="field body"
                bind:value={selected.template}
                spellcheck="false"></textarea>
              <div class="vars">
                {#each variables as token}
                  <button class="chip" onclick={() => insertVariable(token)}>
                    {token}
                  </button>
                {/each}
              </div>
            </div>

            <div class="row">
              <div class="row-label">Font</div>
              <select class="field" bind:value={selected.font}>
                {#each FONTS as [value, name]}
                  <option {value}>{name}</option>
                {/each}
              </select>
            </div>

            <div class="row">
              <div class="row-label">Size</div>
              <div class="slider">
                <input
                  type="range"
                  min="5"
                  max="72"
                  bind:value={selected.points}
                  style="--fill: {((selected.points - 5) / 67) * 100}%"
                  aria-label="Point size" />
                <span class="slider-value">{selected.points} pt</span>
              </div>
            </div>

            <div class="row">
              <div class="row-label">Align</div>
              <div class="segmented" role="group" aria-label="Alignment">
                {#each ALIGN as [value, name]}
                  <button
                    type="button"
                    aria-pressed={selected.align === value}
                    onclick={() => (selected.align = value)}>{name}</button>
                {/each}
              </div>
            </div>

            <div class="row">
              <div class="row-label">Colour</div>
              <input type="color" bind:value={selected.colour} aria-label="Text colour" />
            </div>
          {:else if selected.kind === "image"}
            <div class="row">
              <div>
                <div class="row-label">Column</div>
                <div class="row-helper">
                  Holds a path to the image for each ticket, so a sponsor logo
                  can differ by tier.
                </div>
              </div>
              <input class="field col" bind:value={selected.column} spellcheck="false" />
            </div>
          {:else if selected.kind === "shape"}
            <div class="row">
              <div class="row-label">Fill</div>
              <div class="swatch-row">
                <input
                  type="color"
                  value={selected.fill ?? "#101014"}
                  oninput={(e) => (selected.fill = e.currentTarget.value)}
                  aria-label="Fill colour" />
                <button class="text-button" onclick={() => (selected.fill = null)}>
                  None
                </button>
              </div>
            </div>
            <div class="row">
              <div class="row-label">Outline</div>
              <div class="swatch-row">
                <input
                  type="color"
                  value={selected.stroke ?? "#101014"}
                  oninput={(e) => (selected.stroke = e.currentTarget.value)}
                  aria-label="Outline colour" />
                <button class="text-button" onclick={() => (selected.stroke = null)}>
                  None
                </button>
              </div>
            </div>
          {/if}

          {#if selected.kind === "qr"}
            <div class="row">
              <div>
                <div class="row-label">Size</div>
                <div class="row-helper">
                  Share of the page width, including the light margin the code
                  needs to stay scannable.
                </div>
              </div>
              <div class="slider">
                <input
                  type="range"
                  min="5"
                  max="60"
                  value={Math.round(selected.size * 100)}
                  oninput={(e) => (selected.size = Number(e.currentTarget.value) / 100)}
                  style="--fill: {((selected.size * 100 - 5) / 55) * 100}%"
                  aria-label="Code size" />
                <span class="slider-value">{Math.round(selected.size * 100)}%</span>
              </div>
            </div>
          {:else if selected.kind !== "text"}
            <div class="row">
              <div class="row-label">Width and height</div>
              <div class="slider">
                <input
                  type="range"
                  min="1"
                  max="100"
                  value={Math.round(selected.width * 100)}
                  oninput={(e) => (selected.width = Number(e.currentTarget.value) / 100)}
                  style="--fill: {selected.width * 100}%"
                  aria-label="Width" />
                <span class="slider-value">{Math.round(selected.width * 100)}%</span>
              </div>
            </div>
            <div class="row">
              <div class="row-label">Height</div>
              <div class="slider">
                <input
                  type="range"
                  min="0"
                  max="100"
                  value={Math.round(selected.height * 1000) / 10}
                  oninput={(e) => (selected.height = Number(e.currentTarget.value) / 100)}
                  style="--fill: {selected.height * 100}%"
                  aria-label="Height" />
                <span class="slider-value">{(selected.height * 100).toFixed(1)}%</span>
              </div>
            </div>
          {/if}

          <div class="row">
            <div class="row-label">Rotation</div>
            <div class="slider">
              <input
                type="range"
                min="-180"
                max="180"
                bind:value={selected.rotation}
                style="--fill: {((selected.rotation + 180) / 360) * 100}%"
                aria-label="Rotation" />
              <span class="slider-value">{selected.rotation}&deg;</span>
            </div>
          </div>

          <div class="row">
            <div class="row-label">Opacity</div>
            <div class="slider">
              <input
                type="range"
                min="5"
                max="100"
                value={Math.round(selected.opacity * 100)}
                oninput={(e) => (selected.opacity = Number(e.currentTarget.value) / 100)}
                style="--fill: {selected.opacity * 100}%"
                aria-label="Opacity" />
              <span class="slider-value">{Math.round(selected.opacity * 100)}%</span>
            </div>
          </div>

          <div class="row">
            <div class="row-label">Position</div>
            <span class="hint mono">
              {(selected.x * 100).toFixed(1)}%, {(selected.y * 100).toFixed(1)}%
            </span>
          </div>
        </div>

        <div class="actions">
          <button class="text-button" onclick={() => reorder(-1)}>Back</button>
          <button class="text-button" onclick={() => reorder(1)}>Forward</button>
          <button class="text-button" onclick={duplicate}>Duplicate</button>
          <button class="text-button danger" onclick={remove}>Remove</button>
        </div>
      </div>
    {:else}
      <p class="hint pick">Pick something on the ticket to change it.</p>
    {/if}
  </div>
</div>

<style>
  .layout {
    display: flex;
    flex-wrap: wrap;
    gap: 22px 26px;
    margin-top: 16px;
    align-items: flex-start;
  }
  .stage-wrap {
    flex: 1 1 300px;
    min-width: 260px;
    max-width: 360px;
  }
  .stage-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-faint);
    margin-bottom: 6px;
  }
  .stage {
    position: relative;
    /* Never wider than the artwork needs, never wider than the pane has. */
    width: min(360px, 100%);
    border: 1px solid var(--hairline-strong);
    border-radius: 6px;
    overflow: hidden;
    background: #ffffff;
    touch-action: none;
    user-select: none;
  }
  .stage canvas {
    display: block;
    width: 100%;
    height: 100%;
  }

  .handle {
    position: absolute;
    padding: 0;
    border: 1.5px solid var(--accent);
    background: rgba(34, 136, 191, 0.16);
    border-radius: 3px;
    cursor: grab;
    /* The PDF turns each element about the origin it is drawn from, which is
       the bottom-left of the box in every case. */
    transform-origin: left bottom;
  }
  .handle:active {
    cursor: grabbing;
  }
  .handle.on {
    box-shadow: 0 0 0 2px var(--accent-soft);
    z-index: 2;
  }
  .handle.qr {
    aspect-ratio: 1;
  }
  /* Outline rather than border, and no padding: both would move the box off
     the baseline point it is meant to be showing you. */
  .handle.text {
    border: none;
    outline: 1.5px dashed var(--accent);
    background: rgba(34, 136, 191, 0.1);
    white-space: nowrap;
    line-height: 1;
  }
  /* A rule can be a fraction of a millimetre tall, which is correct on paper
     and impossible to grab on screen. This widens the target without changing
     what is drawn. */
  .handle.shape::after {
    content: "";
    position: absolute;
    inset: -5px;
  }
  .preview-text {
    font-family: var(--font);
    line-height: 1;
  }
  .handle.image {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--accent);
  }
  /* A hint of module texture, so the box reads as a code rather than a
     generic selection rectangle. */
  .grid {
    position: absolute;
    inset: 14%;
    background-image: linear-gradient(var(--accent) 1px, transparent 1px),
      linear-gradient(90deg, var(--accent) 1px, transparent 1px);
    background-size: 25% 25%;
    opacity: 0.35;
  }

  .panel {
    flex: 1 1 340px;
    min-width: 0;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 8px 8px;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--text);
    font-family: inherit;
    font-size: 12.5px;
    text-align: left;
    cursor: pointer;
    transition: background 0.12s ease;
  }
  .item:hover {
    background: var(--hover);
  }
  .item.on {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .item-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .add-row,
  .actions {
    display: flex;
    gap: 2px;
    flex-wrap: wrap;
    margin-left: -8px;
    margin-top: 6px;
  }
  .danger {
    color: var(--bad);
  }
  .danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--bad) 14%, transparent);
  }

  .row.stack {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
  .body {
    min-height: 64px;
    font-size: 12.5px;
    font-family: var(--font-mono);
  }
  .col {
    width: 160px;
  }

  /* Clicking a name beats remembering whether it is {{First Name}} or
     {{first_name}}. */
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
    cursor: pointer;
    transition: background 0.12s ease, color 0.12s ease;
  }
  .chip:hover {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .pick {
    margin-top: 14px;
  }
  .err {
    color: var(--bad);
    margin-top: 6px;
  }
  .warn {
    color: var(--warn);
    margin-top: 6px;
  }
</style>
