<script lang="ts">
  import { ConnectError } from '@connectrpc/connect';
  import { Button, Card, Chip, Status } from '@omnarc/ui';
  import { api, cluster } from '$lib/cluster.svelte';
  import { deviceName, formatGib, gib } from '$lib/model';
  import { currentTheme, setTheme, type Theme } from '$lib/theme';

  let theme = $state<Theme>(currentTheme());
  const themes: { value: Theme; label: string }[] = [
    { value: 'system', label: 'Like the system' },
    { value: 'dark', label: 'Dark' },
    { value: 'light', label: 'Light' },
  ];

  function pick(value: Theme) {
    theme = value;
    setTheme(value);
  }

  // the cluster's name and relays: an admin changes them, once the cluster agrees
  let name = $state('');
  let relay = $state('');
  let busy = $state(false);
  let failed = $state('');
  $effect(() => {
    name = cluster.view.name;
  });

  // what this device offers the cluster (Q1): the plan never uses more
  const me = $derived(cluster.view.devices[cluster.view.thisNode]);
  const totalGib = $derived(me ? gib(me.memoryTotalBytes) : 0);
  let offerGib = $state(0);
  let cores = $state(0);
  let limitHours = $state(false);
  let from = $state('19:00');
  let to = $state('08:00');
  let saved = $state('');
  let loaded = false;
  $effect(() => {
    if (loaded || !me) return;
    loaded = true;
    const budget = me.budget;
    // no budget named yet: half of the memory, as the service offers by default
    offerGib = budget && budget.memoryBytes > 0n ? gib(budget.memoryBytes) : Math.round(totalGib * 4) / 8;
    cores = budget?.cpuCores ?? 0;
    const window = budget?.hours[0];
    limitHours = !!window;
    if (window) {
      from = clock(window.startMinute);
      to = clock(window.endMinute);
    }
  });
  function clock(minute: number): string {
    return `${String(Math.floor(minute / 60)).padStart(2, '0')}:${String(minute % 60).padStart(2, '0')}`;
  }
  function minutes(time: string): number {
    const [h, m] = time.split(':').map(Number);
    return h * 60 + m;
  }
  async function saveBudget() {
    busy = true;
    failed = '';
    saved = '';
    try {
      await api.setBudget({
        budget: {
          memoryBytes: BigInt(Math.round(offerGib * 2 ** 30)),
          cpuCores: cores,
          hours: limitHours ? [{ startMinute: minutes(from), endMinute: minutes(to), weekdays: 0 }] : [],
        },
      });
      saved = 'Saved: the cluster plans with it from now on.';
    } catch (e) {
      failed = e instanceof ConnectError ? e.rawMessage : String(e);
    } finally {
      busy = false;
    }
  }

  async function update(request: { name?: string; addRelay?: string; removeRelay?: string }) {
    busy = true;
    failed = '';
    try {
      await api.updateCluster(request);
      if (request.addRelay) relay = '';
    } catch (e) {
      failed = e instanceof ConnectError ? e.rawMessage : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Settings · {cluster.view.name || 'Omnarc'}</title></svelte:head>

<div class="grid max-w-[720px] gap-5">
  {#if cluster.view.canInvite}
    <Card title="Cluster">
      <div class="grid gap-5">
        <form
          class="grid gap-2"
          onsubmit={(event) => {
            event.preventDefault();
            void update({ name });
          }}
        >
          <label class="grid gap-1.5 text-13 text-text-2" for="cluster-name">Its name</label>
          <div class="flex flex-wrap gap-2">
            <input
              id="cluster-name"
              class="h-10 min-w-0 flex-1 rounded-lg border border-line-2 bg-surface-2 px-3 text-14 text-text-1"
              bind:value={name}
              maxlength="64"
              required
            />
            <Button type="submit" label="Rename" disabled={busy || name.trim() === cluster.view.name} />
          </div>
        </form>

        <div class="grid gap-2">
          <h3 class="m-0 text-13 font-normal text-text-2">Relays, for devices on other networks</h3>
          {#if cluster.view.relays.length}
            <ul class="m-0 grid list-none gap-1.5 p-0">
              {#each cluster.view.relays as url (url)}
                <li class="flex items-center gap-3 rounded-lg border border-line bg-surface-2 px-3 py-2">
                  <span class="min-w-0 flex-1 truncate font-mono text-12 text-text-1">{url}</span>
                  <Button size="sm" variant="ghost" label="Remove" aria-label="Remove the relay {url}" disabled={busy} onclick={() => update({ removeRelay: url })} />
                </li>
              {/each}
            </ul>
          {:else}
            <p class="m-0 text-12 text-text-3">None: the devices meet only on networks they share.</p>
          {/if}
          <form
            class="flex flex-wrap gap-2"
            onsubmit={(event) => {
              event.preventDefault();
              void update({ addRelay: relay });
            }}
          >
            <input
              class="h-10 min-w-0 flex-1 rounded-lg border border-line-2 bg-surface-2 px-3 font-mono text-12 text-text-1"
              bind:value={relay}
              placeholder="https://relay.example.com?token=…"
              aria-label="A relay to add"
              required
            />
            <Button type="submit" label="Add" disabled={busy} />
          </form>
          <p class="m-0 text-12 text-text-3">New invitations name the relays at once; each device uses them from its next start.</p>
        </div>
        {#if failed}<div role="alert"><Status kind="critical" label={failed} /></div>{/if}
      </div>
    </Card>
  {/if}

  <Card title="Theme">
    <div class="flex flex-wrap gap-2" role="radiogroup" aria-label="Theme">
      {#each themes as option (option.value)}
        <button
          class="cursor-pointer border-0 bg-transparent p-0"
          role="radio"
          aria-checked={theme === option.value}
          onclick={() => pick(option.value)}
        >
          <Chip label={option.label} selected={theme === option.value} />
        </button>
      {/each}
    </div>
  </Card>

  {#if me}
    <Card title="What this device offers">
      <form
        class="grid gap-4"
        onsubmit={(event) => {
          event.preventDefault();
          void saveBudget();
        }}
      >
        <div class="grid gap-2">
          <label class="flex items-baseline justify-between text-13 text-text-2" for="offer">
            Memory <span class="font-mono text-12 text-text-1">{offerGib.toFixed(2)} of {formatGib(me.memoryTotalBytes)}</span>
          </label>
          <input id="offer" type="range" min="0.25" max={Math.max(0.25, totalGib)} step="0.25" bind:value={offerGib} />
        </div>
        <label class="flex flex-wrap items-center gap-3 text-13 text-text-2">
          Cores
          <input
            class="h-9 w-20 rounded-lg border border-line-2 bg-surface-2 px-2 text-13 text-text-1"
            type="number"
            min="0"
            max={me.cpuCores || 256}
            bind:value={cores}
          />
          <span>0: all {me.cpuCores || ''}</span>
        </label>
        <div class="flex flex-wrap items-center gap-3 text-13 text-text-2">
          <label class="flex items-center gap-2"><input type="checkbox" bind:checked={limitHours} /> Only from</label>
          <input class="h-9 rounded-lg border border-line-2 bg-surface-2 px-2 text-13 text-text-1" type="time" bind:value={from} disabled={!limitHours} aria-label="From" />
          to
          <input class="h-9 rounded-lg border border-line-2 bg-surface-2 px-2 text-13 text-text-1" type="time" bind:value={to} disabled={!limitHours} aria-label="To" />
        </div>
        <div class="flex flex-wrap items-center gap-3">
          <Button type="submit" variant="primary" label="Save" disabled={busy} />
          {#if saved}<span role="status" class="text-13 text-text-2">{saved}</span>{/if}
        </div>
        <p class="m-0 text-12 text-text-3">
          The cluster never uses more than this. Outside the hours this device offers nothing; a model it holds layers of
          moves on to the others.
        </p>
      </form>
    </Card>
  {/if}

  <Card title="This device">
    <dl class="m-0 grid gap-3 text-13">
      <div class="grid gap-1 sm:grid-cols-[160px_1fr]">
        <dt class="text-text-3">Name</dt>
        <dd class="m-0">{cluster.view.thisNode ? deviceName(cluster.view, cluster.view.thisNode) : '—'}</dd>
      </div>
      <div class="grid gap-1 sm:grid-cols-[160px_1fr]">
        <dt class="text-text-3">Node id</dt>
        <dd class="m-0 font-mono text-12 break-all text-text-2">{cluster.view.thisNode || '—'}</dd>
      </div>
      <div class="grid gap-1 sm:grid-cols-[160px_1fr]">
        <dt class="text-text-3">Cluster</dt>
        <dd class="m-0">{cluster.view.name || '—'}</dd>
      </div>
      <div class="grid gap-1 sm:grid-cols-[160px_1fr]">
        <dt class="text-text-3">Cluster id</dt>
        <dd class="m-0 font-mono text-12 break-all text-text-2">{cluster.view.id || '—'}</dd>
      </div>
    </dl>
    <p class="m-0 mt-4 text-12 text-text-3">
      Nothing is sent anywhere by default. This panel talks only to this device's service, and only this device can open it.
    </p>
  </Card>
</div>
