// The live cluster: a snapshot from this device's service, then its stream of changes, reconnecting when it breaks.

import { createClient } from '@connectrpc/connect';
import { createConnectTransport } from '@connectrpc/connect-web';
import { toasts } from '@omnarc/ui';
import { ClusterService, type WatchClusterResponse } from './gen/omnarc/v1/cluster_service_pb';
import { apply, deviceName, emptyView, hex, type ClusterView } from './model';

const transport = createConnectTransport({ baseUrl: location.origin, useBinaryFormat: true });
export const api = createClient(ClusterService, transport);

class LiveCluster {
  view = $state<ClusterView>(emptyView());
  /** Whether the stream from this device's service is open. */
  connected = $state(false);
  /** Ticks every second, so "online" and countdowns stay current. */
  now = $state(Date.now());
  /** Devices whose joining the owner confirmed in this panel: its dialog shows them join, so no notice repeats it. */
  readonly expected = new Set<string>();
  private started = false;

  start(): void {
    if (this.started) return;
    this.started = true;
    setInterval(() => (this.now = Date.now()), 1_000);
    void this.watch();
  }

  // A notice for what the owner did not see happen: a device joined or was removed (not on a first snapshot).
  private notice(response: WatchClusterResponse): void {
    const event = response.event;
    if (!this.view.id) return;
    if (event.case === 'deviceJoined') {
      const id = hex(event.value.id);
      if (id && !this.view.devices[id] && !this.expected.delete(id) && event.value.name) {
        toasts.show({ kind: 'good', title: `${event.value.name} joined`, body: 'It appears on the ring with its share.' });
      }
    } else if (event.case === 'deviceRemoved') {
      const name = deviceName(this.view, hex(event.value));
      toasts.show({ kind: 'info', title: `${name} was removed`, body: 'The cluster lets it in no more.' });
    } else if (event.case === 'planChanged') {
      const before = this.view.plan;
      const after = event.value;
      if (before && after.version <= before.version) return;
      if (!after.stages.length) {
        if (before) toasts.show({ kind: 'info', title: 'The model stopped', body: 'Every device freed what it held for it.' });
      } else if (!before) {
        const devices = after.stages.length === 1 ? 'one device' : `${after.stages.length} devices`;
        toasts.show({ kind: 'good', title: `${after.model?.name || 'The model'} runs`, body: `Its layers are on ${devices}.` });
      } else {
        // a device of the ring left: the plan solved for its leaving took over
        const gone = before.stages.map((s) => hex(s.node)).filter((id) => !after.stages.some((s) => hex(s.node) === id));
        if (gone.length) {
          toasts.show({
            kind: 'warning',
            title: `${deviceName(this.view, gone[0])} left`,
            body: 'Its standby took its layers; the model goes on.',
          });
        }
      }
    }
  }

  private async watch(): Promise<void> {
    for (;;) {
      try {
        for await (const response of api.watchCluster({})) {
          this.connected = true;
          this.notice(response);
          this.view = apply(this.view, response, Date.now());
        }
      } catch {
        // the service stopped or is restarting
      }
      this.connected = false;
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
  }
}

export const cluster = new LiveCluster();
