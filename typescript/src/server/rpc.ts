const CALCULATOR_CAP_ID = 1;
const CHAT_CAPABILITY_ID = 2;
const SESSION_CAPABILITY_START = 10_000;

type DurableObjectStateWithStorage = {
  storage: {
    get<T>(key: string): Promise<T | undefined>;
    put<T>(key: string, value: T): Promise<void>;
  };
};

type SessionInfo = {
  username: string;
  displayName?: string;
};

type NickTokenInfo = {
  username: string;
  nickname?: string;
  issuedAt: number;
  lastUsed?: number;
};

interface ChatState {
  credentials: Record<string, string>;
  messages: Array<{ from: string; body: string; timestamp: number }>;
  nextSessionCapId: number;
  sessionCaps: Record<string, SessionInfo>;
  registeredNicks: Record<string, string>;
  nickOwners: Record<string, string>;
  nickTokens: Record<string, NickTokenInfo>;
}

const DEFAULT_CHAT_STATE: ChatState = {
  credentials: {
    alice: "password123",
    bob: "hunter2",
    carol: "letmein",
  },
  messages: [],
  nextSessionCapId: SESSION_CAPABILITY_START,
  sessionCaps: {},
  registeredNicks: {},
  nickOwners: {},
  nickTokens: {},
};

function cloneDefaultChatState(): ChatState {
  return {
    credentials: { ...DEFAULT_CHAT_STATE.credentials },
    messages: [...DEFAULT_CHAT_STATE.messages],
    nextSessionCapId: DEFAULT_CHAT_STATE.nextSessionCapId,
    sessionCaps: { ...DEFAULT_CHAT_STATE.sessionCaps },
    registeredNicks: { ...DEFAULT_CHAT_STATE.registeredNicks },
    nickOwners: { ...DEFAULT_CHAT_STATE.nickOwners },
    nickTokens: { ...DEFAULT_CHAT_STATE.nickTokens },
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

  const sessionCaps: Record<string, SessionInfo> = {};
  if (source.sessionCaps && typeof source.sessionCaps === "object") {
    for (const [key, value] of Object.entries(source.sessionCaps as Record<string, unknown>)) {
      if (value && typeof value === "object") {
        const username = (value as Record<string, unknown>).username;
        const displayName = (value as Record<string, unknown>).displayName;
        if (typeof username === "string") {
          sessionCaps[key] = {
            username,
            ...(typeof displayName === "string" ? { displayName } : {}),
          };
        }
      }
    }
  }

  const registeredNicks: Record<string, string> = { ...base.registeredNicks };
  if (source.registeredNicks && typeof source.registeredNicks === "object") {
    for (const [key, value] of Object.entries(source.registeredNicks as Record<string, unknown>)) {
      if (typeof value === "string") {
        registeredNicks[key] = value;
      }
    }
  }

  const nickOwners: Record<string, string> = { ...base.nickOwners };
  if (source.nickOwners && typeof source.nickOwners === "object") {
    for (const [key, value] of Object.entries(source.nickOwners as Record<string, unknown>)) {
      if (typeof value === "string") {
        nickOwners[key] = value;
      }
    }
  }

  const nickTokens: Record<string, NickTokenInfo> = {};
  if (source.nickTokens && typeof source.nickTokens === "object") {
    for (const [key, value] of Object.entries(source.nickTokens as Record<string, unknown>)) {
      if (value && typeof value === "object") {
        const entry = value as Record<string, unknown>;
        const username = typeof entry.username === "string" ? entry.username : null;
        if (!username) {
          continue;
        }
        const nickname = typeof entry.nickname === "string" ? entry.nickname : undefined;
        const issuedAt = typeof entry.issuedAt === "number" && Number.isFinite(entry.issuedAt)
          ? entry.issuedAt
          : Date.now();
        const lastUsed = typeof entry.lastUsed === "number" && Number.isFinite(entry.lastUsed)
          ? entry.lastUsed
          : undefined;
        nickTokens[key] = {
          username,
          ...(nickname ? { nickname } : {}),
          issuedAt,
          ...(lastUsed !== undefined ? { lastUsed } : {}),
        };
      }
    }
  }

  return {
    credentials,
    messages,
    nextSessionCapId,
    sessionCaps,
    registeredNicks,
    nickOwners,
    nickTokens,
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

async function tryHandleChatBatch(payload: string, state: DurableObjectStateWithStorage): Promise<Response | null> {
  console.log('tryHandleChatBatch called with payload:', payload);
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
        if (args.length < 1 || typeof args[0] !== "string") {
          payloadResult = { success: false, message: "`auth` expects <username>" };
          break;
        }

        const username = args[0] as string;

        let sessionCapId = chatState.nextSessionCapId;
        while (chatState.sessionCaps[String(sessionCapId)]) {
          sessionCapId += 1;
        }
        chatState.nextSessionCapId = sessionCapId + 1;
        const storedUsername = `guest-${sessionCapId}`;
        chatState.sessionCaps[String(sessionCapId)] = {
          username: storedUsername,
        };
        mutated = true;

        payloadResult = {
          success: true,
          value: {
            session: {
              _type: "capability",
              id: sessionCapId,
            },
            user: storedUsername,
          },
        };
        break;
      }
      case "redeemNickToken": {
        if (args.length !== 1 || typeof args[0] !== "string") {
          payloadResult = { success: false, message: "`redeemNickToken` expects <token>" };
          break;
        }

        const token = (args[0] as string).trim();
        if (!token) {
          payloadResult = {
            success: true,
            value: {
              status: "error",
              message: "Token must be provided",
            },
          };
          break;
        }

        const tokenInfo = chatState.nickTokens[token];
        if (!tokenInfo) {
          payloadResult = {
            success: true,
            value: {
              status: "error",
              message: "Token not recognized",
            },
          };
          break;
        }

        let sessionCapId = chatState.nextSessionCapId;
        while (chatState.sessionCaps[String(sessionCapId)]) {
          sessionCapId += 1;
        }
        chatState.nextSessionCapId = sessionCapId + 1;
        chatState.sessionCaps[String(sessionCapId)] = {
          username: tokenInfo.username,
          displayName: tokenInfo.nickname ?? tokenInfo.username,
        };

        chatState.nickTokens[token] = {
          ...tokenInfo,
          lastUsed: Date.now(),
        };
        mutated = true;

        payloadResult = {
          success: true,
          value: {
            status: "ok",
            session: {
              _type: "capability",
              id: sessionCapId,
            },
            user: tokenInfo.username,
            nickname: tokenInfo.nickname ?? tokenInfo.username,
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
          console.log('sendMessage case reached with args:', args);
          if (args.length !== 1 || typeof args[0] !== "string") {
            payloadResult = { success: false, message: "`sendMessage` expects <message>" };
            break;
          }
          const message = args[0] as string;
          console.log('Processing sendMessage with message:', message);
          const from = sessionInfo.displayName ?? sessionInfo.username;
          const newMessage = {
            from,
            body: message,
            timestamp: Date.now(),
          };
          chatState.messages.push(newMessage);
          mutated = true;
          
          // Broadcast the message to all connected clients
          try {
            const server = (globalThis as any).serverInstance as any;
            console.log('Server instance found:', !!server);
            console.log('BroadcastMessage method exists:', !!server?.broadcastMessage);
            if (server && server.broadcastMessage) {
              console.log('Broadcasting message:', newMessage);
              await server.broadcastMessage(newMessage);
              console.log('Broadcast completed');
            } else {
              console.log('No server instance or broadcastMessage method available');
            }
          } catch (error) {
            console.error('Failed to broadcast message:', error);
          }
          
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
        case "registerNick": {
          if (args.length !== 2 || typeof args[0] !== "string" || typeof args[1] !== "string") {
            payloadResult = { success: false, message: "`registerNick` expects <nickname>, <password>" };
            break;
          }

          const [nickname, password] = args as [string, string];
          if (chatState.registeredNicks[nickname]) {
            payloadResult = {
              success: true,
              value: {
                status: "error",
                message: "Nickname already registered",
              },
            };
            break;
          }

          chatState.registeredNicks[nickname] = password;
          chatState.nickOwners[nickname] = sessionInfo.username;
          sessionInfo.displayName = nickname;
          chatState.sessionCaps[String(capabilityId)] = sessionInfo;
          mutated = true;

          payloadResult = {
            success: true,
            value: {
              status: "ok",
              message: `Nickname '${nickname}' registered successfully`,
            },
          };
          break;
        }
        case "identifyNick": {
          if (args.length !== 2 || typeof args[0] !== "string" || typeof args[1] !== "string") {
            payloadResult = { success: false, message: "`identifyNick` expects <nickname>, <password>" };
            break;
          }

          const [nickname, password] = args as [string, string];
          const storedPassword = chatState.registeredNicks[nickname];
          if (!storedPassword) {
            payloadResult = {
              success: true,
              value: {
                status: "error",
                message: "Nickname not registered",
              },
            };
            break;
          }

          if (storedPassword !== password) {
            payloadResult = {
              success: true,
              value: {
                status: "error",
                message: "Invalid password",
              },
            };
            break;
          }

          const owner = chatState.nickOwners[nickname];
          if (owner && owner !== sessionInfo.username) {
            console.log(`identifyNick ownership transfer for '${nickname}' from '${owner}' to '${sessionInfo.username}'`);
          }

          chatState.nickOwners[nickname] = sessionInfo.username;
          sessionInfo.displayName = nickname;
          chatState.sessionCaps[String(capabilityId)] = sessionInfo;
          mutated = true;

          payloadResult = {
            success: true,
            value: {
              status: "ok",
              message: `Successfully identified as '${nickname}'`,
            },
          };
          break;
        }
        case "checkNick": {
          if (args.length !== 1 || typeof args[0] !== "string") {
            payloadResult = { success: false, message: "`checkNick` expects <nickname>" };
            break;
          }

          const nickname = args[0] as string;
          payloadResult = {
            success: true,
            value: {
              status: "ok",
              registered: !!chatState.registeredNicks[nickname],
            },
          };
          break;
        }
        case "storeNickToken": {
          if (args.length !== 1 || typeof args[0] !== "string") {
            payloadResult = { success: false, message: "`storeNickToken` expects <token>" };
            break;
          }

          const token = (args[0] as string).trim();
          if (!token) {
            payloadResult = {
              success: true,
              value: {
                status: "error",
                message: "Token must be provided",
              },
            };
            break;
          }

          const now = Date.now();
          chatState.nickTokens[token] = {
            username: sessionInfo.username,
            ...(sessionInfo.displayName ? { nickname: sessionInfo.displayName } : {}),
            issuedAt: now,
            lastUsed: now,
          };

          const tokensForUser = Object.entries(chatState.nickTokens)
            .filter(([, info]) => info.username === sessionInfo.username)
            .sort(([, a], [, b]) => (b.issuedAt ?? 0) - (a.issuedAt ?? 0));

          tokensForUser.slice(5).forEach(([tokenKey]) => {
            delete chatState.nickTokens[tokenKey];
          });

          mutated = true;
          payloadResult = {
            success: true,
            value: {
              status: "ok",
              message: 'Nickname token stored',
            },
          };
          break;
        }
        case "whoami": {
          payloadResult = {
            success: true,
            value: { username: sessionInfo.displayName ?? sessionInfo.username },
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

function handleCalculatorCall(payload: any): any {
  const path = payload[2];
  if (!Array.isArray(path)) {
    throw new Error('call operation must include a method path array');
  }

  const method = path[0];
  if (typeof method !== 'string') {
    throw new Error('call method name must be a string');
  }

  const args = payload[3] || [];

  switch (method) {
    case 'add': {
      if (args.length !== 2 || typeof args[0] !== 'number' || typeof args[1] !== 'number') {
        throw new Error('`add` expects exactly two numeric arguments');
      }
      return args[0] + args[1];
    }
    case 'stats': {
      // This is handled by tryHandleStatsBatch
      throw new Error('stats should be handled by tryHandleStatsBatch');
    }
    default:
      throw new Error(`unknown calculator method \`${method}\``);
  }
}

export async function processRpcBatch(input: string, state: DurableObjectStateWithStorage): Promise<string> {
  // Try stats batch first
  const statsResponse = await tryHandleStatsBatch(input, state);
  if (statsResponse) {
    return await statsResponse.text();
  }

  // Try chat batch
  const chatResponse = await tryHandleChatBatch(input, state);
  if (chatResponse) {
    return await chatResponse.text();
  }

  // Handle calculator operations
  try {
    const lines = input
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line.length > 0);

    if (lines.length === 2) {
      const pushOp = JSON.parse(lines[0]);
      const pullOp = JSON.parse(lines[1]);

      if (Array.isArray(pushOp) && pushOp[0] === "push" && Array.isArray(pullOp) && pullOp[0] === "pull") {
        const callOp = pushOp[1];
        if (Array.isArray(callOp) && callOp[0] === "call" && callOp[1] === CALCULATOR_CAP_ID) {
          const result = handleCalculatorCall(callOp);
          const responseLine = JSON.stringify(["result", pullOp[1], result]);

          const nextCount = ((await state.storage.get<number>("callCount")) ?? 0) + 1;
          await Promise.all([
            state.storage.put("callCount", nextCount),
            state.storage.put("lastRequest", input),
            state.storage.put("lastResponse", responseLine),
          ]);

          return responseLine;
        }
      }
    }

    throw new Error("Unsupported operation");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return JSON.stringify(['error', 0, { message }]);
  }
}
