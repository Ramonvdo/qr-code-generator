<script>
  let { data = $bindable() } = $props();
</script>

<label class="input-label" for="ssid">Network name</label>
<input
  id="ssid"
  class="field js-first-field"
  bind:value={data.ssid}
  spellcheck="false"
  autocomplete="off"
  placeholder="Cafe Guest" />

<div class="section">
  <p class="section-caption">Network</p>
  <div class="rows">
    <div class="row">
      <div>
        <div class="row-label">Security</div>
        <div class="row-helper">
          WPA covers WPA2 and WPA3. Pick Open for a network with no password.
        </div>
      </div>
      <select class="field" bind:value={data.security}>
        <option value="wpa">WPA / WPA2 / WPA3</option>
        <option value="wep">WEP</option>
        <option value="nopass">Open, no password</option>
      </select>
    </div>

    {#if data.security !== "nopass"}
      <div class="row">
        <div>
          <div class="row-label">Password</div>
          <div class="row-helper">
            Anyone who scans the code joins the network, so treat the printed
            code as the password itself.
          </div>
        </div>
        <input
          class="field pw"
          bind:value={data.password}
          spellcheck="false"
          autocomplete="off"
          type="text" />
      </div>
    {/if}

    <div class="row">
      <div>
        <div class="row-label">Hidden network</div>
        <div class="row-helper">
          Only for a network that does not broadcast its name.
        </div>
      </div>
      <label class="toggle">
        <input type="checkbox" bind:checked={data.hidden} />
        <span class="toggle-track"><span class="toggle-knob"></span></span>
      </label>
    </div>
  </div>
</div>

<style>
  .pw {
    width: 200px;
  }
</style>
