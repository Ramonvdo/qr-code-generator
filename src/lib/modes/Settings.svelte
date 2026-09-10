<script>
  import Icon from "../Icon.svelte";

  let { config = $bindable(), busy, onTest } = $props();

  let testTo = $state("");
  let testResult = $state(null);

  let ready = $derived(!!config.apiKey?.trim() && !!config.from?.trim());

  async function test() {
    testResult = null;
    testResult = await onTest(testTo.trim());
  }
</script>

<label class="input-label" for="apikey">Sending</label>
<p class="hint intro">
  Everything else in this app is local. Sending is the one exception, and it
  only happens when you press Send: the buyer's address, the message and their
  ticket go to Resend, and nothing else ever leaves the machine. Your signing
  keys never do.
</p>

<div class="section">
  <p class="section-caption">Resend account</p>
  <div class="rows">
    <div class="row">
      <div>
        <div class="row-label">API key</div>
        <div class="row-helper">
          From the Resend dashboard. Stored in plain text in this app's config
          folder, so treat it like any other saved password.
        </div>
      </div>
      <input
        id="apikey"
        class="field key js-first-field"
        type="password"
        bind:value={config.apiKey}
        spellcheck="false"
        autocomplete="off"
        placeholder="re_..." />
    </div>

    <div class="row">
      <div>
        <div class="row-label">From</div>
        <div class="row-helper">
          Must be on a domain you have verified with Resend. An unverified
          domain fails every message the same way, which is what the test below
          is for.
        </div>
      </div>
      <input
        class="field addr"
        bind:value={config.from}
        spellcheck="false"
        autocomplete="off"
        placeholder="Tickets &lt;tickets@yourdomain.com&gt;" />
    </div>

    <div class="row">
      <div>
        <div class="row-label">Reply to</div>
        <div class="row-helper">Optional. Where replies should go instead.</div>
      </div>
      <input
        class="field addr"
        bind:value={config.replyTo}
        spellcheck="false"
        autocomplete="off"
        placeholder="hello@yourdomain.com" />
    </div>
  </div>
</div>

<div class="section">
  <p class="section-caption">Test</p>
  <div class="rows">
    <div class="row">
      <div>
        <div class="row-label">Send one message to yourself</div>
        <div class="row-helper">
          Worth doing before a real run. A wrong from-address fails all five
          hundred identically, and finding that out once is cheaper.
        </div>
      </div>
      <div class="swatch-row">
        <input
          class="field addr"
          bind:value={testTo}
          spellcheck="false"
          autocomplete="off"
          placeholder="you@example.com" />
        <button class="btn" disabled={busy || !ready || !testTo.trim()} onclick={test}>
          Send test
        </button>
      </div>
    </div>
  </div>

  {#if testResult}
    <p class="result {testResult.ok ? 'good' : 'bad'}">
      <Icon name={testResult.ok ? "check" : "alert"} />
      <span>{testResult.ok ? "Sent. Check your inbox." : testResult.error}</span>
    </p>
  {/if}
</div>

{#if !ready}
  <p class="hint spaced">
    Until a key and a from address are set, the Send button on an event stays
    off and no network code runs at all.
  </p>
{/if}

<style>
  .intro {
    max-width: 520px;
    margin-bottom: 4px;
  }
  .key,
  .addr {
    width: 240px;
  }
  .spaced {
    margin-top: 16px;
    max-width: 520px;
  }
  .result {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    font-size: 12.5px;
    line-height: 1.45;
  }
  .result.good {
    color: var(--good);
  }
  .result.bad {
    color: var(--bad);
  }
</style>
