/**
 * Subscribes to `/dashboard/api/stream/telemetry` and returns the latest
 * `TelemetrySnapshot`. The bearer token is injected by Caddy on the proxy
 * hop, so no client-side auth is required — `EventSource` connects plain.
 *
 * `EventSource` auto-reconnects after network errors with a built-in
 * exponential backoff, so the hook doesn't manage retry state.
 */
import { useEffect, useState } from 'react';
import type { TelemetrySnapshot } from '../types';

const SSE_URL = '/dashboard/api/stream/telemetry';

export function useTelemetryStream(): TelemetrySnapshot | null {
  const [snapshot, setSnapshot] = useState<TelemetrySnapshot | null>(null);

  useEffect(() => {
    const es = new EventSource(SSE_URL);

    es.onmessage = (e: MessageEvent<string>) => {
      try {
        setSnapshot(JSON.parse(e.data) as TelemetrySnapshot);
      } catch {
        // Ignore malformed frames — the sampler emits `: busy` comments
        // when it can't acquire a read lock, and those arrive as empty
        // data. Nothing to do but wait for the next frame.
      }
    };

    return () => es.close();
  }, []);

  return snapshot;
}
