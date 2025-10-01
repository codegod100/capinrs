import initWasm, { process_rpc } from '../../../worker/wasm/capinrs_wasm.js';
// Import the compiled wasm binary URL so we can pass it to the initializer
// This avoids "Invalid URL string" in Miniflare/Workers when init() guesses a path
// Wrangler supports importing .wasm as a module URL
// @ts-ignore - module bundler provides the URL
import wasmUrl from '../../../worker/wasm/capinrs_wasm_bg.wasm';
import { newWebSocketRpcSession, RpcTarget } from 'capnweb';
console.log("WAT")

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const id = env.CAPNWEB.idFromName("global");
    const stub = env.CAPNWEB.get(id);
    return stub.fetch(request);
  },
};

export class CapnWebDurable extends RpcTarget {
  private clients: Set<ChatClientStub> = new Set();
  private sessions: Map<number, ChatSession> = new Map();
  private wasmInitialized = false;

  constructor(private readonly state: DurableObjectStateWithStorage, private readonly env: Env) {
    super();
    (globalThis as any).serverInstance = this;
  }

  private async ensureWasmInitialized() {
    if (!this.wasmInitialized) {
      await initWasm(wasmUrl as unknown as { module_or_path: any });
      this.wasmInitialized = true;
    }
  }

  async fetch(request: Request): Promise<Response> {
    // Handle WebSocket upgrade with capnweb
    if (request.headers.get('Upgrade') === 'websocket') {
      const pair = new WebSocketPair();
      const [serverSocket, clientSocket] = [pair[0], pair[1]];
      serverSocket.accept();

      console.log('Creating WebSocket RPC session with WASM backend');
      const clientStub = newWebSocketRpcSession<ChatClientStub>(serverSocket, this);
      console.log('Client stub created, registering client');
      this.registerClient(clientStub);
      console.log('Client registered, total clients:', this.clients.size);

      return new Response(null, {
        status: 101,
        webSocket: clientSocket,
      });
    }

    // Handle regular HTTP requests using WASM
    return handleRpcWithWasm(request, this.state);
  }

  // RPC methods that clients can call
  async auth(username: string, password: string) {
    console.log('[wasm-auth] called', { username, pwLen: password?.length ?? 0 });
    console.time('[wasm-auth] total');
    
    // Use WASM for authentication
    await this.ensureWasmInitialized();
    
    const payloadLines = [
      JSON.stringify(["push", ["call", 2, ["auth"], [username, password]]]),
      JSON.stringify(["pull", 1]),
    ];
    const payload = payloadLines.join("\n");
    console.log('[wasm-auth] payload', payload);
    try {
      let result: string;
      try {
        result = process_rpc(payload);
      } catch (err) {
        // WASM may throw strings; normalize to Error and log context
        const norm = (typeof err === 'string') ? new Error(err) : (err instanceof Error ? err : new Error(String(err)));
        console.error('[wasm-auth] process_rpc threw', {
          typeOf: typeof err,
          value: err,
          toString: String(err),
        });
        throw norm;
      }
      console.log('[wasm-auth] process_rpc result (string length)', result?.length, 'preview:', result?.slice(0, 200));
      let response: unknown;
      try {
        response = JSON.parse(result);
      } catch (e) {
        console.error('[wasm-auth] JSON.parse failed', { error: String(e), resultPreview: result?.slice(0, 200) });
        throw e;
      }
      console.log('[wasm-auth] parsed response', response);
      if (Array.isArray(response) && response[0] === 'result') {
        // Successful auth via WASM; now return a real capability for `session`
        const sessionId = Math.floor(10000 + Math.random() * 1000000);
        const session = new ChatSession(this.state, username, sessionId, this);
        this.sessions.set(sessionId, session);
        console.log('[wasm-auth] created session capability', { sessionId, username });
        console.timeEnd('[wasm-auth] total');
        return { session, user: username } as any;
      }
      const errVal = Array.isArray(response) ? (response as any)[2] : response;
      const message = (errVal && typeof errVal === 'object' && 'message' in (errVal as any)) ? (errVal as any).message : 'Unknown auth error';
      console.error('[wasm-auth] non-result response', response);
      throw new Error(message);
    } catch (err) {
      const e = (typeof err === 'string') ? new Error(err) : (err instanceof Error ? err : new Error(String(err)));
      console.error('[wasm-auth] threw', {
        name: e.name,
        message: e.message,
        stack: e.stack,
        typeOf: typeof err,
        raw: err,
      });
      console.timeEnd('[wasm-auth] total');
      throw e;
    }
  }

  async sendMessage(capabilityId: number, message: string) {
    console.log('[wasm-sendMessage] called', { capabilityId, msgLen: message?.length ?? 0 });
    
    // Use WASM for message processing
    await this.ensureWasmInitialized();
    
    const payloadLines = [
      JSON.stringify(["push", ["call", capabilityId, ["sendMessage"], [message]]]),
      JSON.stringify(["pull", capabilityId]),
    ];
    const payload = payloadLines.join("\n");
    console.log('[wasm-sendMessage] payload', payload);
    try {
      const result = process_rpc(payload);
      console.log('[wasm-sendMessage] process_rpc result (string length)', result?.length, 'preview:', result?.slice(0, 200));
      const response = JSON.parse(result);
      console.log('[wasm-sendMessage] parsed response', response);
      if (Array.isArray(response) && response[0] === 'result') {
        // Broadcast to all connected clients
        for (const clientStub of Array.from(this.clients)) {
          try {
            await clientStub.receiveMessage({
              from: 'user',
              body: message,
              timestamp: Date.now(),
            });
          } catch (error) {
            console.error('[wasm-sendMessage] failed to send to client', error);
            this.clients.delete(clientStub);
          }
        }
        return (response as any)[2];
      }
      const errVal = Array.isArray(response) ? (response as any)[2] : response;
      const messageText = (errVal && typeof errVal === 'object' && 'message' in (errVal as any)) ? (errVal as any).message : 'Unknown sendMessage error';
      console.error('[wasm-sendMessage] non-result response', response);
      throw new Error(messageText);
    } catch (err) {
      console.error('[wasm-sendMessage] threw', {
        name: (err as any)?.name,
        message: (err as any)?.message,
        stack: (err as any)?.stack,
        typeOf: typeof err,
      });
      throw err;
    }
  }

  async receiveMessages(capabilityId: number) {
    console.log('Server receiveMessages called with capabilityId:', capabilityId);
    
    // Use WASM for message retrieval
    const payloadLines = [
      JSON.stringify(["push", ["call", capabilityId, ["receiveMessages"], []]]),
      JSON.stringify(["pull", capabilityId]),
    ];
    const payload = payloadLines.join("\n");
    
    const result = process_rpc(payload);
    const response = JSON.parse(result);
    
    if (response[0] === "result") {
      return response[2];
    } else {
      throw new Error(response[2].message);
    }
  }

  async whoami(capabilityId: number) {
    console.log('Server whoami called with capabilityId:', capabilityId);
    
    // Use WASM for whoami
    const payloadLines = [
      JSON.stringify(["push", ["call", capabilityId, ["whoami"], []]]),
      JSON.stringify(["pull", capabilityId]),
    ];
    const payload = payloadLines.join("\n");
    
    const result = process_rpc(payload);
    const response = JSON.parse(result);
    
    if (response[0] === "result") {
      return response[2];
    } else {
      throw new Error(response[2].message);
    }
  }

  async broadcastMessage(message: { from: string; body: string; timestamp: number }) {
    console.log(`Broadcasting message to ${this.clients.size} clients:`, message);
    // Broadcast to all connected clients
    for (const clientStub of Array.from(this.clients)) {
      try {
        console.log('Calling receiveMessage on client stub');
        await clientStub.receiveMessage(message);
        console.log('Successfully called receiveMessage on client');
      } catch (error) {
        console.error('Failed to send message to client:', error);
        this.clients.delete(clientStub);
      }
    }
  }

  private registerClient(clientStub: ChatClientStub) {
    console.log('Registering client stub');
    this.clients.add(clientStub);
    console.log('Client added to set, total clients:', this.clients.size);

    if (typeof clientStub.onRpcBroken === 'function') {
      console.log('Setting up RPC broken handler');
      clientStub.onRpcBroken(() => {
        console.log('RPC connection broken, removing client');
        this.clients.delete(clientStub);
      });
    }
  }
}

async function handleRpcWithWasm(request: Request, state: DurableObjectStateWithStorage): Promise<Response> {
  // Initialize WASM module
  await initWasm(wasmUrl as unknown as { module_or_path: any });
  if (request.method === 'GET') {
    const stats = await readDurableStats(state);
    return new Response(JSON.stringify(stats), {
      status: 200,
      headers: {
        "content-type": "application/json",
      },
    });
  }

  if (request.method !== 'POST') {
    return new Response(
      JSON.stringify({
        error: "Send a POST request with Cap'n Web batch payload (text/plain).",
        example: [
          '["push", ["call", 1, ["add"], [10, 20]]',
          '["pull", 1]',
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
    // Use WASM to process the RPC batch
    const responseBody = process_rpc(payload);
    return new Response(responseBody, {
      status: 200,
      headers: {
        "content-type": "text/plain; charset=utf-8",
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

// Type definitions
export interface Env {
  CAPNWEB: DurableObjectNamespace;
}

type DurableObjectStateWithStorage = {
  storage: {
    get<T>(key: string): Promise<T | undefined>;
    put<T>(key: string, value: T): Promise<void>;
  };
};

type ChatClientStub = {
  receiveMessage(message: { from: string; body: string; timestamp: number }): Promise<void> | void;
  onRpcBroken?(callback: (error: unknown) => void): void;
};

class ChatSession {
  constructor(
    private state: DurableObjectStateWithStorage,
    private username: string,
    private capabilityId: number,
    private server: CapnWebDurable,
  ) {}

  // Expose RPC-callable methods for the session capability
  async sendMessage(message: string) {
    return this.server.sendMessage(this.capabilityId, message);
  }

  async receiveMessages() {
    return this.server.receiveMessages(this.capabilityId);
  }

  async whoami() {
    return this.server.whoami(this.capabilityId);
  }
}

