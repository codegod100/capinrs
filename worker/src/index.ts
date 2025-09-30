import initWasm, { process_rpc } from "../wasm/capinrs_wasm.js";
import calculatorWasm from "../wasm/capinrs_wasm_bg.wasm";

let wasmReady: Promise<unknown> | null = null;

async function ensureWasm() {
  if (!wasmReady) {
    wasmReady = initWasm({ module_or_path: calculatorWasm });
  }
  await wasmReady;
}

export default {
  async fetch(request: Request): Promise<Response> {
    await ensureWasm();

    if (request.method !== "POST") {
      return new Response(
        JSON.stringify({
          error: "Send a POST request with JSON { method, args } to invoke the calculator.",
          example: {
            method: "add",
            args: [2, 3],
          },
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
  const resultJson = process_rpc(payload || "{}");
      return new Response(resultJson, {
        status: 200,
        headers: {
          "content-type": "application/json",
        },
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      return new Response(JSON.stringify({ error: `Execution failed: ${message}` }), {
        status: 500,
        headers: { "content-type": "application/json" },
      });
    }
  },
};
