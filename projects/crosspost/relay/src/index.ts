/**
 * crosspost-relay-bridge — the WS leg the TEE can't do.
 *
 * Polls xcross-9f3.testnet get_outbox {"from":cursor} every minute,
 * publishes each NIP-01 event (already TEE-signed) to Nostr relays over
 * WebSocket, then advances the cursor. Relays dedupe by event id, so
 * at-least-once is safe and idempotent.
 */

export interface Env {
  CURSOR: KVNamespace;
  NEAR_RPC: string;
  CONTRACT: string;
  RELAYS: string;
}

interface NostrEvent {
  id: string;
  pubkey: string;
  created_at: number;
  kind: number;
  tags: string[][];
  content: string;
  sig: string;
}

async function viewContract(env: Env, method: string, args: unknown): Promise<string> {
  const res = await fetch(env.NEAR_RPC, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: "1",
      method: "query",
      params: {
        request_type: "call_function",
        finality: "final",
        account_id: env.CONTRACT,
        method_name: method,
        args_base64: btoa(JSON.stringify(args)),
      },
    }),
  });
  const json: any = await res.json();
  if (json.error) throw new Error(`near rpc: ${JSON.stringify(json.error).slice(0, 200)}`);
  // result is an array of utf-8 bytes
  return new TextDecoder().decode(new Uint8Array(json.result.result));
}

/** Publish one event to one relay; resolves true on ["OK", id, true]. */
function publishToRelay(relay: string, ev: NostrEvent, timeoutMs = 10_000): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false;
    const done = (ok: boolean) => {
      if (settled) return;
      settled = true;
      try { ws.close(); } catch {}
      resolve(ok);
    };
    let ws: WebSocket;
    const timer = setTimeout(() => done(false), timeoutMs);
    try {
      ws = new WebSocket(relay);
    } catch {
      clearTimeout(timer);
      return resolve(false);
    }
    ws.onopen = () => ws.send(JSON.stringify(["EVENT", ev]));
    ws.onmessage = (msg) => {
      try {
        const m = JSON.parse(msg.data as string);
        if (m[0] === "OK" && m[1] === ev.id) {
          clearTimeout(timer);
          done(m[2] === true);
        }
      } catch {}
    };
    ws.onerror = () => { clearTimeout(timer); done(false); };
    ws.onclose = () => { clearTimeout(timer); done(false); };
  });
}

async function publishEvent(relays: string[], ev: NostrEvent): Promise<string[]> {
  const results = await Promise.all(relays.map((r) => publishToRelay(r, ev)));
  return relays.map((r, i) => `${r}:${results[i] ? "ok" : "fail"}`);
}

export default {
  async scheduled(_event: ScheduledEvent, env: Env, ctx: ExecutionContext): Promise<void> {
    const cursor = (await env.CURSOR.get("cursor")) ?? "0";
    const relays = env.RELAYS.split(",").map((s) => s.trim()).filter(Boolean);

    let events: NostrEvent[];
    try {
      const raw = await viewContract(env, "get_outbox", { from: cursor });
      // contract quirk: from>0 yields "[,{...}]" — strip leading comma
      events = JSON.parse(raw.replace(/^\[,/, "["));
    } catch (e) {
      console.log("outbox read failed:", (e as Error).message);
      return;
    }
    if (!Array.isArray(events) || events.length === 0) {
      console.log(`no new events (cursor ${cursor})`);
      return;
    }

    const from = parseInt(cursor, 10);
    for (let i = 0; i < events.length; i++) {
      const ev = events[i];
      if (!ev || typeof ev.id !== "string" || ev.id.length !== 64) {
        console.log(`skipping malformed entry at ${from + i}`);
        continue;
      }
      const res = await publishEvent(relays, ev);
      console.log(`event ${ev.id.slice(0, 12)}… → ${res.join(" ")}`);
    }
    await env.CURSOR.put("cursor", String(from + events.length));
    console.log(`cursor ${cursor} → ${from + events.length}`);
  },

  // manual trigger + status: curl https://…/run
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    if (url.pathname === "/run") {
      await (this as any).scheduled({} as ScheduledEvent, env, {} as ExecutionContext);
      return new Response("run complete — see logs");
    }
    const cursor = (await env.CURSOR.get("cursor")) ?? "0";
    return new Response(JSON.stringify({ cursor, contract: env.CONTRACT, relays: env.RELAYS }), {
      headers: { "Content-Type": "application/json" },
    });
  },
};
