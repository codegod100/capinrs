/// <reference path="./global.d.ts" />

import initWasm, { process_rpc } from "../wasm/capinrs_wasm.js";
import calculatorWasm from "../wasm/capinrs_wasm_bg.wasm";

const CHAT_CAPABILITY_ID = 2;
const SESSION_CAPABILITY_START = 10_000;

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

type ChatState = {
  credentials: Record<string, string>;
  messages: Array<{ from: string; body: string; timestamp: number }>;
  nextSessionCapId: number;
  sessionCaps: Record<string, { username: string }>;
};

const DEFAULT_CHAT_STATE: ChatState = {
  credentials: {
    alice: "password123",
    bob: "hunter2",
    carol: "letmein",
  },
  messages: [],
  nextSessionCapId: SESSION_CAPABILITY_START,
  sessionCaps: {},
};

function cloneDefaultChatState(): ChatState {
  return {
    credentials: { ...DEFAULT_CHAT_STATE.credentials },
    messages: [...DEFAULT_CHAT_STATE.messages],
    nextSessionCapId: DEFAULT_CHAT_STATE.nextSessionCapId,
    sessionCaps: { ...DEFAULT_CHAT_STATE.sessionCaps },
  };
}

function normalizeChatState(parsed: unknown): ChatState {
  const base = cloneDefaultChatState();

  if (!parsed || typeof parsed !== "object") {
    return base;
  }

  const source = parsed as Record<string, unknown>;
  const credentials: Record<string, string> = { ...base.credentials };

  if (source.credentials && typeof source.credentials === "object") {
    for (const [key, value] of Object.entries(source.credentials as Record<string, unknown>)) {
      if (typeof value === "string") {
        credentials[key] = value;
      }
    }
  }

  const messages: ChatState["messages"] = [];
  if (Array.isArray(source.messages)) {
    for (const entry of source.messages) {
      if (entry && typeof entry === "object") {
        const record = entry as Record<string, unknown>;
        const from = typeof record.from === "string" ? record.from : null;
        const body = typeof record.body === "string" ? record.body : null;
        const timestamp = typeof record.timestamp === "number" ? record.timestamp : Date.now();
        if (from && body) {
          messages.push({ from, body, timestamp });
        }
      }
    }
  } else {
    messages.push(...base.messages);
  }

  let nextSessionCapId = base.nextSessionCapId;
  if (typeof source.nextSessionCapId === "number" && Number.isFinite(source.nextSessionCapId)) {
    nextSessionCapId = Math.max(
      SESSION_CAPABILITY_START,
      Math.floor(source.nextSessionCapId)
    );
  }

  const sessionCaps: Record<string, { username: string }> = {};
  if (source.sessionCaps && typeof source.sessionCaps === "object") {
    for (const [key, value] of Object.entries(source.sessionCaps as Record<string, unknown>)) {
      if (value && typeof value === "object") {
        const username = (value as Record<string, unknown>).username;
        if (typeof username === "string") {
          sessionCaps[key] = { username };
        }
      }
    }
  }

  return {
    credentials,
    messages,
    nextSessionCapId,
    sessionCaps,
  };
}

async function loadChatState(state: DurableObjectStateWithStorage): Promise<ChatState> {
  const raw = await state.storage.get<string>("chatState");
  if (!raw) {
    return cloneDefaultChatState();
  }

  try {
    return normalizeChatState(JSON.parse(raw));
  } catch {
    return cloneDefaultChatState();
  }
}

async function persistChatState(state: DurableObjectStateWithStorage, chatState: ChatState) {
  await state.storage.put("chatState", JSON.stringify(chatState));
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

async function tryHandleChatBatch(payload: string, state: DurableObjectStateWithStorage): Promise<Response | null> {
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

  if (!Array.isArray(pullOp) || pullOp[0] !== "pull") {
    return null;
  }

  const importId = (pullOp as unknown[])[1];
  if (typeof importId !== "number") {
    return null;
  }

  const callOp = (pushOp as unknown[])[1];
  if (!Array.isArray(callOp) || callOp[0] !== "call") {
    return null;
  }

  const capabilityId = (callOp as unknown[])[1];
  if (typeof capabilityId !== "number") {
    return null;
  }

  const path = (callOp as unknown[])[2];
  const method = Array.isArray(path) && typeof path[0] === "string" ? path[0] : null;
  if (!method) {
    return null;
  }

  const args = (callOp as unknown[])[3];
  if (!Array.isArray(args)) {
    return null;
  }

  const chatState = await loadChatState(state);
  let mutated = false;
  let payloadResult:
    | { success: true; value: unknown }
    | { success: false; message: string }
    | null = null;

  const persistMessages = () =>
    chatState.messages.map((msg) => ({
      from: msg.from,
      body: msg.body,
      timestamp: msg.timestamp,
    }));

  if (capabilityId === CHAT_CAPABILITY_ID) {
    switch (method) {
      case "auth": {
        if (args.length !== 2 || typeof args[0] !== "string" || typeof args[1] !== "string") {
          payloadResult = { success: false, message: "`auth` expects <username>, <password>" };
          break;
        }

        const [username, password] = args as [string, string];
        const stored = chatState.credentials[username];
        if (!stored || stored !== password) {
          payloadResult = { success: false, message: "invalid credentials" };
          break;
        }

        let sessionCapId = chatState.nextSessionCapId;
        while (chatState.sessionCaps[String(sessionCapId)]) {
          sessionCapId += 1;
        }
        chatState.nextSessionCapId = sessionCapId + 1;
        chatState.sessionCaps[String(sessionCapId)] = { username };
        mutated = true;

        payloadResult = {
          success: true,
          value: {
            session: {
              _type: "capability",
              id: sessionCapId,
            },
            user: username,
          },
        };
        break;
      }
      case "sendMessage":
      case "receiveMessages": {
        payloadResult = {
          success: false,
          message: "Call this method on the session capability returned by `auth`",
        };
        break;
      }
      default: {
        payloadResult = {
          success: false,
          message: `Unknown chat method: ${method}`,
        };
        break;
      }
    }
  } else {
    const sessionInfo = chatState.sessionCaps[String(capabilityId)];
    if (!sessionInfo) {
      payloadResult = { success: false, message: "unknown session capability" };
    } else {
      switch (method) {
        case "sendMessage": {
          if (args.length !== 1 || typeof args[0] !== "string") {
            payloadResult = { success: false, message: "`sendMessage` expects <message>" };
            break;
          }
          const message = args[0] as string;
          chatState.messages.push({
            from: sessionInfo.username,
            body: message,
            timestamp: Date.now(),
          });
          mutated = true;
          payloadResult = {
            success: true,
            value: {
              status: "ok",
              echo: message,
            },
          };
          break;
        }
        case "receiveMessages": {
          if (args.length !== 0) {
            payloadResult = { success: false, message: "`receiveMessages` takes no arguments" };
            break;
          }
          payloadResult = {
            success: true,
            value: {
              messages: persistMessages(),
            },
          };
          break;
        }
        case "whoami": {
          payloadResult = {
            success: true,
            value: { username: sessionInfo.username },
          };
          break;
        }
        default: {
          payloadResult = {
            success: false,
            message: `method ${method} not supported on session capability`,
          };
          break;
        }
      }
    }
  }

  if (!payloadResult) {
    return null;
  }

  if (mutated) {
    await persistChatState(state, chatState);
  }

  const reply = payloadResult.success
    ? ["result", importId, payloadResult.value]
    : ["error", importId, { message: payloadResult.message }];

  const responseLine = JSON.stringify(reply);
  const nextCount = ((await state.storage.get<number>("callCount")) ?? 0) + 1;
  await Promise.all([
    state.storage.put("callCount", nextCount),
    state.storage.put("lastRequest", payload),
    state.storage.put("lastResponse", responseLine),
  ]);

  return new Response(responseLine, {
    status: 200,
    headers: {
      "content-type": "text/plain; charset=utf-8",
      "x-capnweb-call-count": String(nextCount),
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

  const chatResponse = await tryHandleChatBatch(payload, state);
  if (chatResponse) {
    return chatResponse;
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
