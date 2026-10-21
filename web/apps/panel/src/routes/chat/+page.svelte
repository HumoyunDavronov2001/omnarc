<script lang="ts">
  import { ConnectError } from '@connectrpc/connect';
  import { Button, Card, EmptyState } from '@omnarc/ui';
  import { api, cluster } from '$lib/cluster.svelte';
  import type { AskDone } from '$lib/gen/omnarc/v1/cluster_service_pb';
  import { ringPath } from '$lib/model';

  const view = $derived(cluster.view);
  const path = $derived(ringPath(view));

  interface Turn {
    prompt: string;
    answer: string;
    done: AskDone | null;
    failed: string;
  }
  let turns = $state<Turn[]>([]);
  let prompt = $state('');
  let asking = $state(false);
  // accuracy mode (EN-23): several sampled paths, the answer most agree on
  let accurate = $state(false);

  async function ask() {
    const text = prompt.trim();
    if (!text || asking) return;
    prompt = '';
    asking = true;
    // the whole conversation goes along, so the model answers in its own chat template and remembers it
    const messages = turns
      .filter((t) => !t.failed)
      .flatMap((t) => [
        { role: 'user', content: t.prompt },
        { role: 'assistant', content: t.answer },
      ]);
    messages.push({ role: 'user', content: text });
    turns.push({ prompt: text, answer: '', done: null, failed: '' });
    const turn = turns[turns.length - 1];
    try {
      for await (const step of api.ask({ prompt: text, messages, maxTokens: 256, paths: accurate ? 3 : 1 })) {
        if (step.step.case === 'piece') turn.answer += step.step.value;
        else if (step.step.case === 'done') turn.done = step.step.value;
      }
    } catch (e) {
      turn.failed = e instanceof ConnectError ? e.rawMessage : String(e);
    } finally {
      asking = false;
    }
  }

  function timings(done: AskDone): string {
    const speed = done.tpotMs > 0 ? ` · ${(1000 / done.tpotMs).toFixed(1)} tokens/s` : '';
    const agreed = done.paths > 1 ? ` · ${done.agreed} of ${done.paths} paths agreed` : '';
    return `${done.generatedTokens} tokens · first after ${Math.round(done.ttftMs)} ms${speed}${agreed}`;
  }
</script>

<svelte:head><title>Chat · {view.name || 'Omnarc'}</title></svelte:head>

<div class="grid max-w-[880px] gap-5">
  {#if !view.plan}
    <Card>
      <EmptyState icon="chat" title="No model runs" text="Start one in Models; every device of the cluster can ask it then." />
    </Card>
  {:else}
    <Card title={view.plan.model?.name || 'Chat'}>
      <div class="grid gap-4">
        {#if path.length > 1}
          <p class="m-0 text-13 text-text-2">Every token goes {path.join(' → ')}.</p>
        {/if}
        {#each turns as turn, i (i)}
          <div class="grid gap-2">
            <p class="m-0 justify-self-end rounded-[10px] bg-surface-2 px-3 py-2 text-14 text-text-1">{turn.prompt}</p>
            <div class="rounded-[10px] border border-line px-3 py-2">
              <p class="m-0 text-14 whitespace-pre-wrap text-text-1" aria-live="polite">{turn.answer || (turn.failed ? '' : '…')}</p>
              {#if turn.done}
                <p class="m-0 mt-2 font-mono text-12 text-text-3">{timings(turn.done)}</p>
              {/if}
              {#if turn.failed}
                <p role="alert" class="m-0 text-13 text-critical">{turn.failed}</p>
              {/if}
            </div>
          </div>
        {/each}
        <form
          class="flex flex-wrap gap-2"
          onsubmit={(event) => {
            event.preventDefault();
            void ask();
          }}
        >
          <label class="sr-only" for="prompt">Your question</label>
          <textarea
            id="prompt"
            class="min-h-[44px] min-w-0 flex-1 resize-y rounded-lg border border-line-2 bg-surface-2 px-3 py-2.5 text-14 text-text-1"
            rows="2"
            placeholder="Ask the model"
            bind:value={prompt}
            onkeydown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey) {
                event.preventDefault();
                void ask();
              }
            }}
          ></textarea>
          <Button type="submit" variant="primary" icon="chat" label="Ask" disabled={asking || !prompt.trim()} />
        </form>
        <label class="flex items-center gap-2 text-13 text-text-2">
          <input type="checkbox" bind:checked={accurate} />
          Accuracy mode: three paths answer, and the answer most agree on is shown
        </label>
      </div>
    </Card>
  {/if}
</div>
