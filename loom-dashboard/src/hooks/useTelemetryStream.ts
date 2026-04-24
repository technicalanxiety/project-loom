/**
 * Subscribes to `/dashboard/api/stream/telemetry` and returns the latest
 * `TelemetrySnapshot` plus a live connection flag. The bearer token is
 * injected by Caddy on the `/dashboard/api/*` proxy hop, so `EventSource`
 * can connect plain — no client-side auth.
 *
 * `EventSource` auto-reconnects with browser-default exponential backoff,
 * so the hook doesn't manage retry state itself; it only surfaces the
 * current `readyState` so the page can show "connection lost" when the
 * stream is between reconnect attempts.
 */
import { useEffect, useState } from 'react';
import type { TelemetrySnapshot } from '../types';

const SSE_URL = '/dashboard/api/stream/telemetry';

export interface TelemetryStream {
  snapshot: TelemetrySnapshot | null;
  /** True while the EventSource is open. False during reconnect windows. */
  connected: boolean;
}

export function useTelemetryStream(): TelemetryStream {
  const [snapshot, setSnapshot] = useState<TelemetrySnapshot | null>(null);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    const es = new EventSource(SSE_URL);

    es.onopen = () => setConnected(true);

    es.onmessage = (e: MessageEvent<string>) => {
      try {
        setSnapshot(JSON.parse(e.data) as TelemetrySnapshot);
      } catch {
        // Ignored — the server may emit malformed frames during startup
        // before the first sampler tick completes.
      }
    };

    es.onerror = () => setConnected(false);

    return () => es.close();
  }, []);

  return { snapshot, connected };
}
