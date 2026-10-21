<script lang="ts">
  import '../app.css';
  import { page } from '$app/state';
  import { Button, Icon, Logo, Status, Toaster, type IconName } from '@omnarc/ui';
  import AddDevice from '$lib/AddDevice.svelte';
  import FirstRun from '$lib/FirstRun.svelte';
  import { cluster } from '$lib/cluster.svelte';
  import { isOnline } from '$lib/model';

  let { children } = $props();
  cluster.start();

  let adding = $state(false);
  // a device asking to join opens the dialog, even if the owner closed it
  $effect(() => {
    if (cluster.view.joinAttempt) adding = true;
  });

  const ids = $derived(Object.keys(cluster.view.devices));
  // the service answers, but this device belongs to no cluster yet
  const firstRun = $derived(cluster.connected && !cluster.view.id);
  // once a cluster is created or joined, the ring opens at its top, not where the form was
  let wasFirstRun = false;
  $effect(() => {
    if (wasFirstRun && !firstRun) window.scrollTo(0, 0);
    wasFirstRun = firstRun;
  });
  const online = $derived(ids.filter((id) => isOnline(cluster.view, id, cluster.now)).length);
  const health = $derived(
    online === ids.length
      ? `${ids.length} ${ids.length === 1 ? 'device' : 'devices'} · all online`
      : `${ids.length} devices · ${ids.length - online} offline`,
  );

  const sections: { href: string; label: string; icon: IconName }[] = [
    { href: '/', label: 'Ring', icon: 'ring' },
    { href: '/devices', label: 'Devices', icon: 'devices' },
    { href: '/models', label: 'Models', icon: 'cube' },
    { href: '/chat', label: 'Chat', icon: 'chat' },
    { href: '/observe', label: 'Observe', icon: 'observe' },
    { href: '/settings', label: 'Settings', icon: 'settings' },
  ];
</script>

<div class="min-h-screen md:grid md:grid-cols-[224px_1fr]">
  <nav class="flex items-center gap-1 border-b border-line bg-surface-1 px-3 md:flex-col md:items-stretch md:border-r md:border-b-0 md:py-4" aria-label="Sections">
    <a class="mr-3 flex items-center gap-2.5 px-2 py-3 text-14 font-[650] tracking-[0.14em] text-text-1 no-underline md:mr-0 md:mb-3" href="/">
      <Logo size={22} />OMNARC
    </a>
    {#each sections as section (section.href)}
      <a
        class="flex h-9 items-center gap-2.5 rounded-lg px-2.5 text-13 text-text-2 no-underline hover:bg-surface-2 hover:text-text-1 aria-[current=page]:bg-surface-2 aria-[current=page]:text-text-1"
        href={section.href}
        aria-current={page.url.pathname === section.href ? 'page' : undefined}
      >
        <Icon name={section.icon} size={18} /><span class="hidden sm:inline">{section.label}</span>
      </a>
    {/each}
  </nav>

  <div class="flex min-w-0 flex-col">
    <header class="flex h-14 items-center gap-3 border-b border-line px-5">
      <h1 class="m-0 min-w-0 truncate text-16 font-semibold">{cluster.view.name || 'Omnarc'}</h1>
      {#if cluster.connected && ids.length && !firstRun}
        <span class="hidden sm:inline-flex"><Status kind={online === ids.length ? 'good' : 'warning'} label={health} /></span>
      {/if}
      <div class="ml-auto shrink-0 whitespace-nowrap">
        {#if cluster.view.canInvite}
          <Button variant="primary" icon="plus" label="Add device" onclick={() => (adding = true)} />
        {/if}
      </div>
    </header>
    {#if !cluster.connected}
      <p role="status" class="m-5 mb-0 rounded-[10px] border border-line-2 bg-surface-1 px-4 py-3 text-13 text-text-2">
        This device's service does not answer yet. Start it with <code class="font-mono text-text-1">omnarc-node serve</code>;
        the panel connects by itself.
      </p>
    {/if}
    <main class="p-5">
      {#if firstRun}
        <FirstRun />
      {:else}
        {@render children()}
      {/if}
    </main>
  </div>
</div>

<AddDevice bind:open={adding} />
<Toaster />
