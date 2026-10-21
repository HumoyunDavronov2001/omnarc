<script lang="ts">
  import { ConnectError } from '@connectrpc/connect';
  import { Button, Card, Chip, EmptyState, Status } from '@omnarc/ui';
  import { api, cluster } from '$lib/cluster.svelte';
  import type { ModelInfo, RunModelResponse } from '$lib/gen/omnarc/v1/cluster_service_pb';
  import { RunMode } from '$lib/gen/omnarc/v1/plan_pb';
  import { deviceName, formatSize, hex, planPart, ringPath } from '$lib/model';

  const view = $derived(cluster.view);

  let models = $state<ModelInfo[]>([]);
  let loading = $state(true);
  let failed = $state('');
  let busy = $state('');
  // the answer to "Will it fit?" for one model, before it starts
  let preview = $state<{ model: string; answer: RunModelResponse } | null>(null);
  let context = $state(4096);
  // the KV cache's element type (EN-16): 8 and 4 bits fit about two and three and a half times the context
  let kvCache = $state('f16');
  // what the plan is chosen for (NO-45): one answer fast, or many people at once
  let mode = $state(RunMode.LATENCY);

  async function load() {
    try {
      models = (await api.listModels({})).models;
      failed = '';
    } catch (e) {
      failed = message(e);
    } finally {
      loading = false;
    }
  }
  void load();

  function message(e: unknown): string {
    return e instanceof ConnectError ? e.rawMessage : String(e);
  }

  async function run(model: ModelInfo, dryRun: boolean) {
    busy = model.blake3;
    failed = '';
    try {
      const answer = await api.runModel({ model: model.blake3, context, dryRun, kvCache, mode });
      preview = dryRun ? { model: model.blake3, answer } : null;
    } catch (e) {
      failed = message(e);
      preview = null;
    } finally {
      busy = '';
    }
  }

  async function stop() {
    busy = 'stop';
    try {
      await api.stopModel({});
    } catch (e) {
      failed = message(e);
    } finally {
      busy = '';
    }
  }

  const reasons: Record<string, string> = {
    too_small: 'offers less than one layer needs',
    not_needed: 'the model runs faster without it',
  };

  function speed(ms: number): string {
    return ms > 0 ? `about ${(1000 / ms).toFixed(ms < 100 ? 0 : 1)} tokens/s` : 'not measured yet';
  }

  const running = $derived(view.plan);

  // a faster plan the cluster found for the running model; a dismissed one stays hidden in this browser
  let dismissed = $state(readDismissed());
  function readDismissed(): string {
    try {
      return localStorage.getItem('omnarc.suggestion.dismissed') ?? '';
    } catch {
      return '';
    }
  }
  const suggestion = $derived(view.suggestion);
  const suggestionKey = $derived(
    suggestion?.plan
      ? suggestion.plan.stages.map((s) => `${hex(s.node)}:${s.layerBegin}-${s.layerEnd}`).join(',')
      : '',
  );
  function dismiss() {
    dismissed = suggestionKey;
    try {
      localStorage.setItem('omnarc.suggestion.dismissed', suggestionKey);
    } catch {
      // a private window keeps no setting: the card is hidden for this visit
    }
  }
  async function apply() {
    const plan = running;
    if (!plan?.model) return;
    busy = 'apply';
    failed = '';
    try {
      const hash = Array.from(plan.model.blake3, (b) => b.toString(16).padStart(2, '0')).join('');
      await api.runModel({ model: hash, context: plan.context, mode: plan.mode });
    } catch (e) {
      failed = message(e);
    } finally {
      busy = '';
    }
  }
  const path = $derived(ringPath(view));
</script>

<svelte:head><title>Models · {view.name || 'Omnarc'}</title></svelte:head>

<div class="grid max-w-[880px] gap-5">
  {#if running && suggestion?.plan && suggestionKey !== dismissed}
    <Card title="A faster plan">
      <div class="grid gap-3">
        <p class="m-0 text-14 text-text-1">
          The devices or what they offer changed: {suggestion.plan.stages.length === 1
            ? 'one device can hold the whole model'
            : `${suggestion.plan.stages.length} devices would share it`}, {speed(suggestion.msPerToken)} instead of
          {speed(suggestion.currentMsPerToken).replace('about ', '')}.
        </p>
        <ul class="m-0 grid list-none gap-1 p-0 text-13 text-text-2">
          {#each suggestion.plan.stages as stage, i (i)}
            <li>{deviceName(view, hex(stage.node))}: layers {stage.layerBegin}–{stage.layerEnd - 1}{stage.gpuLayers ? `, ${stage.gpuLayers} on its GPU` : ''}{i === 0 ? ', answers' : ''}</li>
          {/each}
        </ul>
        <div class="flex flex-wrap gap-2">
          <Button variant="primary" label="Apply" disabled={!!busy} onclick={apply} />
          <Button variant="ghost" label="Dismiss" onclick={dismiss} />
        </div>
        <p class="m-0 text-12 text-text-3">Applying moves the layers between the devices; an answer being generated stops.</p>
      </div>
    </Card>
  {/if}

  {#if running}
    <Card title="Running now">
      <div class="grid gap-4">
        <div class="flex flex-wrap items-center gap-3">
          <Status kind="good" label={running.model?.name || 'A model'} />
          <span class="text-13 text-text-2">
            {running.model?.layerCount} layers · context {running.context}{running.kvCache && running.kvCache !== 'f16'
              ? ` · ${running.kvCache === 'q8_0' ? '8' : '4'}-bit cache`
              : ''} · plan {running.version}
          </span>
          <div class="ml-auto">
            <Button label="Stop" icon="power" disabled={busy === 'stop'} onclick={stop} />
          </div>
        </div>
        <ol class="m-0 grid list-none gap-1.5 p-0" aria-label="Devices of the plan, in ring order">
          {#each Object.keys(view.devices) as id (id)}
            {@const part = planPart(running, id)}
            {@const fetching = view.telemetry[id]?.fetching}
            {#if part}
              <li class="flex items-center gap-3 rounded-lg border border-line bg-surface-2 px-3 py-2 text-13">
                <span class="min-w-0 flex-1 truncate text-text-1">{deviceName(view, id)}</span>
                {#if fetching}
                  <span class="font-mono text-12 text-text-2" role="status">
                    fetching {fetching.totalBytes > 0n
                      ? `${Math.min(100, Math.round((Number(fetching.receivedBytes) / Number(fetching.totalBytes)) * 100))}%`
                      : formatSize(fetching.receivedBytes)}
                  </span>
                {/if}
                <span class="font-mono text-12 text-text-2">layers {part.begin}–{part.end - 1}</span>
                <Chip label={part.role === 'head' ? 'answers' : part.role === 'replica' ? 'stands by' : 'computes'} tone={part.role === 'replica' ? undefined : 'good'} />
              </li>
            {/if}
          {/each}
        </ol>
        {#if path.length > 1}
          <p class="m-0 text-13 text-text-2">A token goes {path.join(' → ')}.</p>
        {/if}
      </div>
    </Card>
  {/if}

  <Card title="Models on this device">
    {#if loading}
      <p class="m-0 text-13 text-text-2">Looking…</p>
    {:else if !models.length}
      <EmptyState
        icon="cube"
        title="No model here yet"
        text="Add a GGUF file on this computer with omnarc-node model add <file>; the other devices fetch their layers from it."
      />
    {:else}
      <div class="grid gap-4">
        <label class="flex flex-wrap items-center gap-3 text-13 text-text-2">
          Context
          <select class="h-9 rounded-lg border border-line-2 bg-surface-2 px-2 text-13 text-text-1" bind:value={context}>
            {#each [2048, 4096, 8192, 16384, 32768] as n (n)}
              <option value={n}>{n.toLocaleString()} tokens</option>
            {/each}
          </select>
          <span>longer contexts need more memory on every device</span>
        </label>
        <label class="flex flex-wrap items-center gap-3 text-13 text-text-2">
          Plan for
          <select class="h-9 rounded-lg border border-line-2 bg-surface-2 px-2 text-13 text-text-1" bind:value={mode}>
            <option value={RunMode.LATENCY}>one answer as fast as it goes</option>
            <option value={RunMode.THROUGHPUT}>many people at once</option>
          </select>
        </label>
        <label class="flex flex-wrap items-center gap-3 text-13 text-text-2">
          KV cache
          <select class="h-9 rounded-lg border border-line-2 bg-surface-2 px-2 text-13 text-text-1" bind:value={kvCache}>
            <option value="f16">16-bit, exact</option>
            <option value="q8_0">8-bit, about half the memory</option>
            <option value="q4_0">4-bit, about a quarter</option>
          </select>
        </label>
        <ul class="m-0 grid list-none gap-3 p-0">
          {#each models as model (model.blake3)}
            <li class="grid gap-3 rounded-[10px] border border-line bg-surface-1 p-4">
              <div class="flex flex-wrap items-center gap-3">
                <span class="min-w-0 flex-1 truncate text-14 font-medium text-text-1">{model.name}</span>
                <span class="font-mono text-12 text-text-2">{formatSize(model.fileBytes)} · {model.layerCount} layers</span>
                {#if !model.complete}<Chip label="only part of it here" tone="warning" />{/if}
              </div>
              <div class="flex flex-wrap gap-2">
                <Button label="Will it fit?" disabled={!!busy || !model.complete} onclick={() => run(model, true)} />
                <Button variant="primary" label={running ? 'Start instead' : 'Start'} disabled={!!busy || !model.complete} onclick={() => run(model, false)} />
              </div>
              {#if preview && preview.model === model.blake3}
                {@const plan = preview.answer.plan}
                <div class="grid gap-2 rounded-lg border border-line-2 bg-surface-2 p-3" role="status">
                  <p class="m-0 text-13 text-text-1">
                    It fits: {plan?.stages.length === 1 ? 'one device holds all of it' : `${plan?.stages.length} devices share it`},
                    {speed(preview.answer.estimatedMsPerToken)}.
                  </p>
                  <ul class="m-0 grid list-none gap-1 p-0 text-13 text-text-2">
                    {#each plan?.stages ?? [] as stage, i (i)}
                      <li>{deviceName(view, hex(stage.node))}: layers {stage.layerBegin}–{stage.layerEnd - 1}{stage.gpuLayers ? `, ${stage.gpuLayers} on its GPU` : ''}{i === 0 ? ', answers' : ''}</li>
                    {/each}
                    {#each plan?.replicas ?? [] as replica, i (i)}
                      <li>{deviceName(view, hex(replica.node))}: stands by for layers {replica.layerBegin}–{replica.layerEnd - 1}{replica.gpuLayers ? `, ${replica.gpuLayers} on its GPU` : ''}</li>
                    {/each}
                    {#each preview.answer.leftOut as out, i (i)}
                      <li>{deviceName(view, hex(out.node))}: not used, {reasons[out.reason] ?? out.reason}</li>
                    {/each}
                  </ul>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      </div>
    {/if}
    {#if failed}
      <p role="alert" class="m-0 mt-3 text-13 text-critical">{failed}</p>
    {/if}
  </Card>
</div>
