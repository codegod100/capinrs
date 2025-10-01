import initWasm, { process_rpc } from "../wasm/capinrs_wasm.js";
import calculatorWasm from "../wasm/capinrs_wasm_bg.wasm";

interface Env {
  CAPNWEB: DurableObjectNamespace;
}

type DurableObjectStateWithStorage = {
  storage: {
    get<T>(key: string): Promise<T | undefined>;
    put<T>(key: string, value: T): Promise<void>;
  };
};

let wasmReady: Promise<unknown> | null = null;

async function ensureWasm() {
  if (!wasmReady) {
    wasmReady = initWasm({ module_or_path: calculatorWasm });
  }
  await wasmReady;
}

async function readDurableStats(state: DurableObjectStateWithStorage) {
  const [callCount, lastRequest, lastResponse] = await Promise.all([
    state.storage.get<number>("callCount"),
    state.storage.get<string>("lastRequest"),
    state.storage.get<string>("lastResponse"),
  ]);

  return {
    callCount: callCount ?? 0,
    lastRequest: lastRequest ?? null,
    lastResponse: lastResponse ?? null,
  };
}

async function tryHandleStatsBatch(payload: string, state: DurableObjectStateWithStorage): Promise<Response | null> {
  const lines = payload
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

  if (lines.length !== 2) {
    return null;
  }

  let pushOp: unknown;
  let pullOp: unknown;

  try {
    pushOp = JSON.parse(lines[0]);
    pullOp = JSON.parse(lines[1]);
  } catch {
    return null;
  }

  if (!Array.isArray(pushOp) || pushOp[0] !== "push") {
    return null;
  }

  const callOp = (pushOp as unknown[])[1];
  if (!Array.isArray(callOp) || callOp[0] !== "call") {
    return null;
  }

  const path = (callOp as unknown[])[2];
  const method = Array.isArray(path) && typeof path[0] === "string" ? path[0] : null;
  if (method !== "stats") {
    return null;
  }

  if (!Array.isArray(pullOp) || pullOp[0] !== "pull") {
    return null;
  }

  const importId = (pullOp as unknown[])[1];
  if (typeof importId !== "number") {
    return null;
  }

  const stats = await readDurableStats(state);
  const responseLine = JSON.stringify(["result", importId, stats]);

  return new Response(responseLine, {
    status: 200,
    headers: {
      "content-type": "text/plain; charset=utf-8",
      "x-capnweb-call-count": String(stats.callCount),
    },
  });
}

async function handleRpc(request: Request, state: DurableObjectStateWithStorage): Promise<Response> {
  if (request.method === "GET") {
    const stats = await readDurableStats(state);

    return new Response(JSON.stringify(stats), {
      status: 200,
      headers: {
        "content-type": "application/json",
      },
    });
  }

  if (request.method !== "POST") {
    return new Response(
      JSON.stringify({
        error: "Send a POST request with Cap'n Web batch payload (text/plain).",
        example: [
          '[\"push\", [\"call\", 1, [\"add\"], [10, 20]]]',
          '[\"pull\", 1]',
        ].join("\n"),
      }),
      {
        status: 405,
        headers: {
          "content-type": "application/json",
          allow: "POST",
        },
      },
    );
  }

  let payload: string;
  try {
    payload = await request.text();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return new Response(JSON.stringify({ error: `Failed to read request body: ${message}` }), {
      status: 400,
      headers: { "content-type": "application/json" },
    });
  }

  const statsResponse = await tryHandleStatsBatch(payload, state);
  if (statsResponse) {
    return statsResponse;
  }

  await ensureWasm();

  try {
    const responseBody = process_rpc(payload);
    const nextCount = ((await state.storage.get<number>("callCount")) ?? 0) + 1;
    await Promise.all([
      state.storage.put("callCount", nextCount),
      state.storage.put("lastRequest", payload),
      state.storage.put("lastResponse", responseBody),
    ]);

    return new Response(responseBody, {
      status: 200,
      headers: {
        "content-type": "text/plain; charset=utf-8",
        "x-capnweb-call-count": nextCount.toString(),
      },
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return new Response(JSON.stringify({ error: message }), {
      status: 400,
      headers: { "content-type": "application/json" },
    });
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const id = env.CAPNWEB.idFromName("global");
    const stub = env.CAPNWEB.get(id);
    return stub.fetch(request);
  },
};

export class CapnWebDurable {
  constructor(private readonly state: DurableObjectStateWithStorage, private readonly env: Env) {}

  async fetch(request: Request): Promise<Response> {
    return handleRpc(request, this.state);
  }
}
