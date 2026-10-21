// The cluster as the panel keeps it, and the pure functions that change and describe it.

import type { RingMarker } from '@omnarc/ui';
import type { ClusterSnapshot, JoinAttempt, PlanSuggestion, WatchClusterResponse } from './gen/omnarc/v1/cluster_service_pb';
import type { PlacementPlan } from './gen/omnarc/v1/plan_pb';
import { Backend, Platform, type DeviceInfo, type NodeId } from './gen/omnarc/v1/device_pb';
import { LinkKind, ThermalState, type LinkSample, type PowerState, type Telemetry } from './gen/omnarc/v1/telemetry_pb';

/** A device is online while its telemetry keeps arriving; it is sent every second. */
export const ONLINE_MS = 5_000;

export interface ClusterView {
  name: string;
  /** The cluster id (hex). */
  id: string;
  /** The node that serves this panel. */
  thisNode: string;
  canInvite: boolean;
  /** The cluster's relays, without their tokens. */
  relays: string[];
  /** Devices by node id (hex), in the order they became known. */
  devices: Record<string, DeviceInfo>;
  telemetry: Record<string, Telemetry>;
  /** When the panel last heard from each device (its own clock, ms); gone once the device leaves. */
  heard: Record<string, number>;
  /** When each device was last heard from, kept after it goes offline: its "last seen". */
  seen: Record<string, number>;
  /** A device that asks to join and waits for the owner. */
  joinAttempt: JoinAttempt | null;
  /** The model the cluster runs and where; null while none runs. */
  plan: PlacementPlan | null;
  /** A faster plan for it, offered by the device that started it (NO-48). */
  suggestion: PlanSuggestion | null;
}

export function emptyView(): ClusterView {
  return {
    name: '',
    id: '',
    thisNode: '',
    canInvite: false,
    relays: [],
    devices: {},
    telemetry: {},
    heard: {},
    seen: {},
    joinAttempt: null,
    plan: null,
    suggestion: null,
  };
}

export function hex(id?: NodeId): string {
  return id ? Array.from(id.ed25519PublicKey, (b) => b.toString(16).padStart(2, '0')).join('') : '';
}

export function short(id: string): string {
  return id.slice(0, 10);
}

/** The view after a snapshot. */
export function fromSnapshot(snapshot: ClusterSnapshot, now: number): ClusterView {
  const view = emptyView();
  view.name = snapshot.name;
  view.id = snapshot.id ? Array.from(snapshot.id.ed25519PublicKey, (b) => b.toString(16).padStart(2, '0')).join('') : '';
  view.thisNode = hex(snapshot.thisNode);
  view.canInvite = snapshot.canInvite;
  view.relays = [...snapshot.relays];
  view.plan = snapshot.plans.find((plan) => plan.stages.length > 0) ?? null;
  view.suggestion = snapshot.suggestion ?? null;
  for (const device of snapshot.devices) view.devices[hex(device.id)] = device;
  for (const telemetry of snapshot.telemetry) {
    const id = hex(telemetry.node);
    view.telemetry[id] = telemetry;
    // the service keeps a device's last telemetry after it goes quiet: it was heard when it was sent
    const sent = Number(telemetry.unixMs);
    view.heard[id] = sent > 0 ? Math.min(now, sent) : now;
    view.seen[id] = view.heard[id];
  }
  return view;
}

/** The view after one event of the live stream; the same object, changed. */
export function apply(view: ClusterView, response: WatchClusterResponse, now: number): ClusterView {
  const event = response.event;
  switch (event.case) {
    case 'snapshot': {
      const next = fromSnapshot(event.value, now);
      // a device that left is in the new snapshot without telemetry; it keeps its last-seen time
      for (const [id, at] of Object.entries(view.seen)) if (next.devices[id] && !(id in next.seen)) next.seen[id] = at;
      return next;
    }
    case 'deviceJoined': {
      const id = hex(event.value.id);
      if (id) view.devices[id] = event.value;
      if (view.joinAttempt && hex(view.joinAttempt.joiner) === id) view.joinAttempt = null;
      break;
    }
    case 'deviceLeft':
      delete view.telemetry[hex(event.value)];
      delete view.heard[hex(event.value)];
      break;
    case 'deviceRemoved':
      delete view.devices[hex(event.value)];
      delete view.telemetry[hex(event.value)];
      delete view.heard[hex(event.value)];
      delete view.seen[hex(event.value)];
      break;
    case 'telemetry': {
      const id = hex(event.value.node);
      view.telemetry[id] = event.value;
      view.heard[id] = now;
      view.seen[id] = now;
      break;
    }
    case 'joinAttempt':
      view.joinAttempt = event.value;
      break;
    case 'planChanged':
      // a plan without stages: the model stopped
      if (!view.plan || event.value.version > view.plan.version) view.plan = event.value.stages.length ? event.value : null;
      break;
    default:
      break;
  }
  return view;
}

export function isOnline(view: ClusterView, id: string, now: number): boolean {
  return now - (view.heard[id] ?? -Infinity) < ONLINE_MS;
}

export function gib(bytes: bigint | number): number {
  return Number(bytes) / 2 ** 30;
}

/** A size in the unit that reads best: MB below a GiB. */
export function formatSize(bytes: bigint | number): string {
  const n = Number(bytes);
  return n < 2 ** 30 ? `${Math.max(1, Math.round(n / 2 ** 20))} MB` : formatGib(bytes);
}

export function formatGib(bytes: bigint | number): string {
  const value = gib(bytes);
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} GiB`;
}

export function platformName(platform: Platform): string {
  switch (platform) {
    case Platform.LINUX:
      return 'Linux';
    case Platform.WINDOWS:
      return 'Windows';
    case Platform.MACOS:
      return 'macOS';
    case Platform.ANDROID:
      return 'Android';
    case Platform.IOS:
      return 'iOS';
    case Platform.BROWSER:
      return 'Browser';
    default:
      return '—';
  }
}

export function platformIcon(platform: Platform): 'desktop' | 'laptop' | 'phone' | 'browser' {
  switch (platform) {
    case Platform.ANDROID:
    case Platform.IOS:
      return 'phone';
    case Platform.BROWSER:
      return 'browser';
    case Platform.MACOS:
      return 'laptop';
    default:
      return 'desktop';
  }
}

/** A compute backend's name, as people know it. */
export function backendName(backend: Backend): string {
  switch (backend) {
    case Backend.CUDA:
      return 'CUDA';
    case Backend.HIP:
      return 'ROCm';
    case Backend.VULKAN:
      return 'Vulkan';
    case Backend.METAL:
      return 'Metal';
    case Backend.OPENCL:
      return 'OpenCL';
    case Backend.HEXAGON:
      return 'Hexagon';
    case Backend.SYCL:
      return 'SYCL';
    case Backend.CANN:
      return 'CANN';
    case Backend.WEBGPU:
      return 'WebGPU';
    case Backend.MLX:
      return 'MLX';
    case Backend.VLLM:
      return 'vLLM';
    case Backend.SGLANG:
      return 'SGLang';
    default:
      return 'CPU';
  }
}

/** What a device computes with: its accelerators' backends, or its processor. */
export function backends(device: DeviceInfo): string {
  const names = [...new Set(device.accelerators.map((unit) => backendName(unit.backend)))];
  return names.length ? names.join(', ') : 'CPU';
}

/** A device's name, or its short id before it has told the cluster what it is. */
export function deviceName(view: ClusterView, id: string): string {
  return view.devices[id]?.name || short(id);
}

/** This device's measured link to the device `id`, from this device's own telemetry. */
export function linkTo(view: ClusterView, id: string): LinkSample | undefined {
  if (id === view.thisNode) return undefined;
  return view.telemetry[view.thisNode]?.links.find((link) => hex(link.peer) === id);
}

/** A link as the panel shows it: `Wi-Fi · 3 ms · 420 Mbit/s`. */
export function formatLink(link: LinkSample): string {
  const kind: Partial<Record<LinkKind, string>> = {
    [LinkKind.WIFI]: 'Wi-Fi',
    [LinkKind.ETHERNET]: 'Ethernet',
    [LinkKind.USB]: 'USB',
    [LinkKind.THUNDERBOLT]: 'Thunderbolt',
    [LinkKind.RELAY]: 'Relay',
    [LinkKind.WEBRTC]: 'WebRTC',
  };
  const ms = link.rttMs < 10 ? link.rttMs.toFixed(1) : Math.round(link.rttMs).toString();
  const parts = [kind[link.kind], `${ms} ms`];
  if (link.bandwidthMbps > 0) {
    parts.push(link.bandwidthMbps >= 1000 ? `${(link.bandwidthMbps / 1000).toFixed(1)} Gbit/s` : `${Math.round(link.bandwidthMbps)} Mbit/s`);
  }
  return parts.filter(Boolean).join(' · ');
}

/** How warm a device runs, in words, with the status it is shown with; nothing when the device does not say. */
export function thermal(state: ThermalState | undefined): { label: string; kind: 'good' | 'warning' | 'serious' | 'critical' } | null {
  switch (state) {
    case ThermalState.NOMINAL:
      return { label: 'Normal', kind: 'good' };
    case ThermalState.FAIR:
      return { label: 'Warm', kind: 'warning' };
    case ThermalState.SERIOUS:
      return { label: 'Hot', kind: 'serious' };
    case ThermalState.CRITICAL:
      return { label: 'Too hot', kind: 'critical' };
    default:
      return null;
  }
}

/** Where a device's power comes from, in words; nothing when the device does not say. */
export function power(state: PowerState | undefined): string | null {
  if (!state) return null;
  const percent = `${Math.round(state.batteryFraction * 100)}%`;
  if (state.charging) return `Charging · ${percent}`;
  if (state.onBattery) return `On battery · ${percent}`;
  return 'Plugged in';
}

/**
 * The thin outer markers of a device's arc on the ring (PLAN.md §9.4): charging, the battery and the temperature,
 * when there is something to say — a device plugged in at a normal temperature has none.
 */
export function markers(telemetry: Telemetry | undefined): RingMarker[] {
  const out: RingMarker[] = [];
  const state = telemetry?.power;
  const supply = power(state);
  if (state && supply && state.charging) out.push({ icon: 'bolt', kind: 'good', label: supply });
  else if (state && supply && state.onBattery) {
    out.push({ icon: 'battery', kind: state.batteryFraction < 0.2 ? 'serious' : 'warning', label: supply });
  }
  const warmth = thermal(telemetry?.thermal);
  if (warmth && warmth.kind !== 'good') out.push({ icon: 'temp', kind: warmth.kind, label: warmth.label });
  return out;
}

/** Seconds since a device was last heard from; nothing if never. */
export function seenAgo(view: ClusterView, id: string, now: number): number | null {
  const at = view.seen[id];
  return at === undefined ? null : Math.max(0, Math.round((now - at) / 1000));
}

/** A time in the past in words: "12 s ago", "5 min ago", "3 h ago", "2 days ago". */
export function formatAgo(seconds: number): string {
  if (seconds < 60) return `${seconds} s ago`;
  if (seconds < 3_600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3_600)} h ago`;
  const days = Math.floor(seconds / 86_400);
  return `${days} ${days === 1 ? 'day' : 'days'} ago`;
}

/** What a device does in the plan: the layers it runs, or the window it stands by for. */
export interface PlanPart {
  role: 'head' | 'stage' | 'replica';
  begin: number;
  end: number;
}

export function planPart(plan: PlacementPlan | null, id: string): PlanPart | null {
  if (!plan) return null;
  const index = plan.stages.findIndex((stage) => hex(stage.node) === id);
  if (index >= 0) {
    const stage = plan.stages[index];
    return { role: index === 0 ? 'head' : 'stage', begin: stage.layerBegin, end: stage.layerEnd };
  }
  const replica = plan.replicas.find((r) => hex(r.node) === id);
  return replica ? { role: 'replica', begin: replica.layerBegin, end: replica.layerEnd } : null;
}

/** "layers 0–15 · head", for a device's line. */
export function describePart(part: PlanPart): string {
  const layers = `layers ${part.begin}–${part.end - 1}`;
  return part.role === 'head' ? `${layers} · answers` : part.role === 'replica' ? `stands by for ${layers}` : layers;
}

/** The path one token takes: the devices of the plan in ring order, back to the head. */
export function ringPath(view: ClusterView): string[] {
  if (!view.plan) return [];
  const names = view.plan.stages.map((stage) => deviceName(view, hex(stage.node)));
  return names.length > 1 ? [...names, names[0]] : names;
}
