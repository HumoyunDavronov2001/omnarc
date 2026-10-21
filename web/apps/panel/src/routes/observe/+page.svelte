<script lang="ts">
  import { Card, EmptyState } from '@omnarc/ui';
  import { cluster } from '$lib/cluster.svelte';
  import { deviceName, hex } from '$lib/model';
  import { LinkKind } from '$lib/gen/omnarc/v1/telemetry_pb';

  const view = $derived(cluster.view);
  const ids = $derived(Object.keys(view.devices));

  // the link from one device to another, as the first one measured it
  function link(from: string, to: string) {
    return view.telemetry[from]?.links.find((l) => hex(l.peer) === to);
  }

  // One token around the ring, from what the devices measured: each device's mean compute per token, then the hop
  // to the next one (half its round trip). It is drawn from measurements, not traced token by token.
  interface Segment {
    device: string;
    kind: 'compute' | 'network';
    start: number;
    ms: number;
  }
  const path = $derived.by(() => {
    const plan = view.plan;
    if (!plan) return { segments: [] as Segment[], total: 0, devices: [] as string[] };
    const devices = plan.stages.map((s) => hex(s.node));
    const segments: Segment[] = [];
    let at = 0;
    devices.forEach((id, i) => {
      const compute = view.telemetry[id]?.stages[0]?.msPerToken ?? 0;
      segments.push({ device: id, kind: 'compute', start: at, ms: compute });
      at += compute;
      if (devices.length > 1) {
        const next = devices[(i + 1) % devices.length];
        const hop = (link(id, next)?.rttMs ?? 0) / 2;
        segments.push({ device: id, kind: 'network', start: at, ms: hop });
        at += hop;
      }
    });
    return { segments, total: at, devices };
  });

  function busy(id: string): number {
    if (!path.total) return 0;
    return path.segments.filter((s) => s.device === id).reduce((sum, s) => sum + s.ms, 0) / path.total;
  }

  // latency colors: the load ramp, fast links light, slow ones dark
  function shade(ms: number): string {
    const level = ms < 1 ? 7 : ms < 3 ? 6 : ms < 8 ? 5 : ms < 20 ? 4 : ms < 50 ? 3 : 2;
    return `var(--load-${level})`;
  }

  const kinds: Partial<Record<LinkKind, string>> = {
    [LinkKind.WIFI]: 'Wi-Fi',
    [LinkKind.ETHERNET]: 'Ethernet',
    [LinkKind.USB]: 'USB',
    [LinkKind.THUNDERBOLT]: 'Thunderbolt',
    [LinkKind.RELAY]: 'Relay',
    [LinkKind.WEBRTC]: 'WebRTC',
  };

  let asTable = $state(false);
</script>

<svelte:head><title>Observe · {view.name || 'Omnarc'}</title></svelte:head>

<div class="grid max-w-[980px] gap-5">
  <Card title="Token path">
    {#if !view.plan}
      <EmptyState icon="observe" title="No model runs" text="Start one in Models to see where a token spends its time." />
    {:else if !path.total}
      <p class="m-0 text-13 text-text-2">The devices have not computed a token yet: ask the model once.</p>
    {:else}
      <div class="grid gap-3">
        <p class="m-0 text-13 text-text-2">
          One token goes around the ring in about <b class="font-mono text-text-1">{path.total.toFixed(1)} ms</b>
          ({(1000 / path.total).toFixed(0)} tokens/s). Drawn from each device's mean compute per token and its measured
          link to the next.
        </p>
        <div class="grid gap-2" role="img" aria-label="Where one token spends its time, device by device">
          {#each path.devices as id (id)}
            <div class="grid grid-cols-[140px_1fr_64px] items-center gap-3 text-13">
              <span class="truncate text-text-1">{deviceName(view, id)}</span>
              <div class="relative h-5 rounded bg-surface-2">
                {#each path.segments.filter((s) => s.device === id && s.ms > 0) as s, i (i)}
                  <span
                    class="absolute top-0 h-5 rounded"
                    style:left="{(s.start / path.total) * 100}%"
                    style:width="{Math.max(0.5, (s.ms / path.total) * 100)}%"
                    style:background="var(--chart-{s.kind})"
                    title="{s.kind === 'compute' ? 'compute' : 'to the next device'}: {s.ms.toFixed(2)} ms"
                  ></span>
                {/each}
              </div>
              <span class="text-right font-mono text-12 text-text-2">{Math.round((1 - busy(id)) * 100)}% idle</span>
            </div>
          {/each}
        </div>
        <div class="flex flex-wrap gap-4 text-12 text-text-3">
          <span class="flex items-center gap-1.5"><i class="inline-block h-2.5 w-2.5 rounded-sm" style:background="var(--chart-compute)"></i>compute</span>
          <span class="flex items-center gap-1.5"><i class="inline-block h-2.5 w-2.5 rounded-sm" style:background="var(--chart-network)"></i>network</span>
          <span class="flex items-center gap-1.5"><i class="inline-block h-2.5 w-2.5 rounded-sm bg-surface-2"></i>idle</span>
        </div>
      </div>
    {/if}
  </Card>

  <Card title="Link latency, ms">
    {#if ids.length < 2}
      <p class="m-0 text-13 text-text-2">Links appear once there are two devices.</p>
    {:else}
      <div class="grid gap-3">
        <button class="justify-self-start cursor-pointer rounded-lg border border-line-2 bg-surface-2 px-3 py-1.5 text-12 text-text-1" onclick={() => (asTable = !asTable)}>
          {asTable ? 'Matrix view' : 'Table view'}
        </button>
        {#if asTable}
          <table class="w-full border-collapse text-12">
            <thead>
              <tr class="text-left text-text-3">
                <th class="py-1.5 font-normal">From</th><th class="py-1.5 font-normal">To</th><th class="py-1.5 font-normal">Kind</th>
                <th class="py-1.5 text-right font-normal">Round trip</th><th class="py-1.5 text-right font-normal">Bandwidth</th>
              </tr>
            </thead>
            <tbody>
              {#each ids as from (from)}
                {#each ids.filter((to) => to !== from && link(from, to)) as to (to)}
                  {@const l = link(from, to)!}
                  <tr class="border-t border-line">
                    <td class="py-1.5 text-text-1">{deviceName(view, from)}</td>
                    <td class="py-1.5 text-text-1">{deviceName(view, to)}</td>
                    <td class="py-1.5 text-text-2">{kinds[l.kind] ?? '—'}</td>
                    <td class="py-1.5 text-right font-mono text-text-2">{l.rttMs.toFixed(1)} ms</td>
                    <td class="py-1.5 text-right font-mono text-text-2">{l.bandwidthMbps ? `${Math.round(l.bandwidthMbps)} Mbit/s` : '—'}</td>
                  </tr>
                {/each}
              {/each}
            </tbody>
          </table>
        {:else}
          <div class="overflow-x-auto">
            <table class="border-collapse text-12" aria-label="Round trip between every two devices, as the row's device measured it">
              <thead>
                <tr>
                  <th></th>
                  {#each ids as to (to)}<th class="max-w-[96px] truncate px-1 pb-1.5 font-normal text-text-3">{deviceName(view, to)}</th>{/each}
                </tr>
              </thead>
              <tbody>
                {#each ids as from (from)}
                  <tr>
                    <th class="max-w-[140px] truncate pr-2 text-left font-normal text-text-1">{deviceName(view, from)}</th>
                    {#each ids as to (to)}
                      {@const l = from === to ? undefined : link(from, to)}
                      <td class="p-0.5">
                        <div
                          class="grid h-9 w-16 place-items-center rounded font-mono {l ? 'text-bg' : 'text-text-3'}"
                          style:background={l ? shade(l.rttMs) : 'var(--surface-2)'}
                        >
                          {from === to ? '' : l ? l.rttMs.toFixed(1) : '—'}
                        </div>
                      </td>
                    {/each}
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          <div class="flex items-center gap-2 text-12 text-text-3">
            fast <span class="flex h-2 w-40 overflow-hidden rounded">{#each [7, 6, 5, 4, 3, 2] as level (level)}<i class="flex-1" style:background="var(--load-{level})"></i>{/each}</span> slow
          </div>
        {/if}
      </div>
    {/if}
  </Card>
</div>
