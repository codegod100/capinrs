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

async function handleRpc(request: Request, state: DurableObjectStateWithStorage): Promise<Response> {
  if (request.method === "GET") {
    const [callCount, lastRequest, lastResponse] = await Promise.all([
      state.storage.get<number>("callCount"),
      state.storage.get<string>("lastRequest"),
      state.storage.get<string>("lastResponse"),
    ]);

    return new Response(
      JSON.stringify({
        callCount: callCount ?? 0,
        lastRequest: lastRequest ?? null,
        lastResponse: lastResponse ?? null,
      }),
      {
        status: 200,
        headers: {
          "content-type": "application/json",
        },
      },
    );
  }

  await ensureWasm();

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
