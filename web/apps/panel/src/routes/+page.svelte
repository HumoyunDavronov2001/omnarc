<script lang="ts">
  import { goto } from '$app/navigation';
  import { Card, Icon, Meter, Ring, Table, Tooltip, loadLevel, type Column } from '@omnarc/ui';
  import { cluster } from '$lib/cluster.svelte';
  import { describePart, deviceName, formatGib, gib, isOnline, markers, planPart, platformIcon, platformName, thermal } from '$lib/model';

  const view = $derived(cluster.view);

  // the same data as a table, for those who would rather read it (PLAN.md §9.4); remembered in this browser
  let asTable = $state(readTable());
  function readTable(): boolean {
    try {
      return localStorage.getItem('omnarc.ring.table') === '1';
    } catch {
      return false;
    }
  }
  function toggleTable() {
    asTable = !asTable;
    try {
      localStorage.setItem('omnarc.ring.table', asTable ? '1' : '0');
    } catch {
      // a private window keeps no setting: the toggle still works for this visit
    }
  }
  const tableColumns: Column[] = [
    { key: 'name', label: 'Device', wrap: true },
    { key: 'share', label: 'Share', align: 'end', mono: true },
    { key: 'load', label: 'Load', align: 'end', mono: true },
    { key: 'state', label: 'State' },
  ];

  // a phone gets the small ring without labels
  let narrow = $state(matchMedia('(max-width: 640px)').matches);
  $effect(() => {
    const query = matchMedia('(max-width: 640px)');
    const change = () => (narrow = query.matches);
    query.addEventListener('change', change);
    return () => query.removeEventListener('change', change);
  });
  const ids = $derived(Object.keys(view.devices));
  const totalBytes = $derived(ids.reduce((sum, id) => sum + Number(view.devices[id].memoryTotalBytes), 0));
  const usedBytes = $derived(ids.reduce((sum, id) => sum + Number(view.telemetry[id]?.memoryUsedBytes ?? 0n), 0));

  // while a model runs, an arc's length is the layers its device computes; until then, its share of the memory
  const arcs = $derived(
    ids.map((id) => {
      const online = isOnline(view, id, cluster.now);
      const load = view.telemetry[id]?.cpuLoad ?? 0;
      const part = planPart(view.plan, id);
      const computes = part && part.role !== 'replica';
      return {
        name: deviceName(view, id),
        layers: view.plan
          ? computes
            ? part.end - part.begin
            : 1
          : Math.max(1, Math.round(gib(view.devices[id].memoryTotalBytes) * 10)),
        level: online ? loadLevel(load) : 1,
        note: `${part ? describePart(part) : formatGib(view.devices[id].memoryTotalBytes)} · ${online ? `${Math.round(load * 100)}% CPU` : 'offline'}`,
        // charging, the battery, the temperature: what an offline device last said no longer holds
        markers: online ? markers(view.telemetry[id]) : [],
      };
    }),
  );
  const summary = $derived(
    `${ids.length} ${ids.length === 1 ? 'device' : 'devices'} with ${formatGib(totalBytes)} of memory together; ` +
      arcs
        .map((arc, i) => {
          const state = `${arc.name} ${isOnline(view, ids[i], cluster.now) ? 'online' : 'offline'}`;
          return arc.markers.length ? `${state} (${arc.markers.map((m) => m.label.toLowerCase()).join(', ')})` : state;
        })
        .join(', ') +
      '.',
  );
</script>

<svelte:head><title>Ring · {view.name || 'Omnarc'}</title></svelte:head>

<div class="grid gap-5 xl:grid-cols-[minmax(0,1fr)_380px]">
  <Card padding="lg">
    {#if ids.length}
      <div class="mb-2 flex justify-end">
        <Tooltip
          text={asTable ? 'Show the ring' : 'Show the ring as a table'}
          label="Table"
          pressed={asTable}
          onclick={toggleTable}
        >
          <Icon name="table" size={18} />
        </Tooltip>
      </div>
      {#if asTable}
        <Table caption={summary} columns={tableColumns} rows={ids} key={(id) => id}>
          {#snippet cell(id, column)}
            {@const online = isOnline(view, id, cluster.now)}
            {@const load = view.telemetry[id]?.cpuLoad ?? 0}
            {#if column.key === 'name'}
              <a class="text-text-1 no-underline hover:underline" href="/devices/{id}">{deviceName(view, id)}</a>
            {:else if column.key === 'share'}
              {totalBytes ? `${Math.round((Number(view.devices[id].memoryTotalBytes) / totalBytes) * 100)}%` : '—'}
            {:else if column.key === 'load'}
              {online ? `${Math.round(load * 100)}%` : '—'}
            {:else}
              {online ? 'Online' : 'Offline'}
            {/if}
          {/snippet}
        </Table>
      {:else}
        <Ring
          variant={narrow ? 'compact' : 'full'}
          devices={arcs}
          label={`${summary} Use the arrow keys to move from device to device; Enter opens one.`}
          center={{ value: String(ids.length), unit: ids.length === 1 ? 'device' : 'devices', caption: `${formatGib(totalBytes)} of memory` }}
          onselect={(i) => goto(`/devices/${ids[i]}`)}
        />
      {/if}
      <p class="m-0 text-center text-13 text-text-3">
        {#if view.plan}
          {view.plan.model?.name || 'A model'} runs: every arc is a device, as long as the layers it computes; its color is
          its load.
        {:else if asTable}
          A device's share is its part of the cluster's memory. No model is running yet.
        {:else}
          Every arc is a device, as long as its share of the cluster's memory; its color is its load. No model is running yet.
        {/if}
      </p>
    {:else}
      <p class="m-0 text-13 text-text-3">Waiting for this device's service…</p>
    {/if}
  </Card>

  <div class="grid content-start gap-5">
    <Card title="Together">
      <dl class="m-0 grid grid-cols-1 gap-4 min-[400px]:grid-cols-2">
        <div>
          <dt class="text-12 text-text-3">Memory in use</dt>
          <dd class="m-0 mt-1 font-mono text-20 whitespace-nowrap">
            {formatGib(usedBytes)} <span class="text-13 text-text-3">/ {formatGib(totalBytes)}</span>
          </dd>
        </div>
        <div>
          <dt class="text-12 text-text-3">Online</dt>
          <dd class="m-0 mt-1 font-mono text-20">
            {ids.filter((id) => isOnline(view, id, cluster.now)).length} <span class="text-13 text-text-3">/ {ids.length}</span>
          </dd>
        </div>
      </dl>
    </Card>

    <Card title="Devices">
      <ul class="m-0 grid list-none gap-1 p-0">
        {#each ids as id (id)}
          {@const device = view.devices[id]}
          {@const telemetry = view.telemetry[id]}
          {@const online = isOnline(view, id, cluster.now)}
          {@const warmth = online ? thermal(telemetry?.thermal) : null}
          <li class="grid grid-cols-[20px_minmax(0,1fr)_72px_44px] items-center gap-3 py-2">
            <span class="text-text-3"><Icon name={platformIcon(device.platform)} size={18} /></span>
            <span class="min-w-0">
              <span class="block truncate text-13 text-text-1">
                {deviceName(view, id)}{#if id === view.thisNode}<span class="text-text-3"> · this device</span>{/if}
              </span>
              <span class="block text-12 text-text-3">
                {platformName(device.platform)} · {online ? 'online' : 'offline'}{warmth && warmth.kind !== 'good'
                  ? ` · ${warmth.label.toLowerCase()}`
                  : ''}
              </span>
            </span>
            <Meter value={online ? (telemetry?.cpuLoad ?? 0) : 0} label={`CPU of ${deviceName(view, id)}`} level={online ? loadLevel(telemetry?.cpuLoad ?? 0) : 1} />
            <span class="text-right font-mono text-12 text-text-2">{online ? `${Math.round((telemetry?.cpuLoad ?? 0) * 100)}%` : '—'}</span>
          </li>
        {/each}
      </ul>
    </Card>
  </div>
</div>
