import { processRpcBatch } from './rpc';
import { newWebSocketRpcSession, RpcTarget } from 'capnweb';

const CALCULATOR_CAP_ID = 1;
const CHAT_CAPABILITY_ID = 2;
const SESSION_CAPABILITY_START = 10_000;

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

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const id = env.CAPNWEB.idFromName("global");
    const stub = env.CAPNWEB.get(id);
    return stub.fetch(request);
  },
};

// RPC target for individual chat sessions
class ChatSession extends RpcTarget {
  private currentNickname: string;

  constructor(
    private state: DurableObjectStateWithStorage,
    private username: string,
    private capabilityId: number
  ) {
    super();
    this.currentNickname = username; // Start with the login username
  }

  async sendMessage(message: string) {
    console.log(`DEBUG: sendMessage called with message='${message}', currentNickname='${this.currentNickname}', username='${this.username}'`);
    const chatState = await loadChatState(this.state);

    const newMessage = {
      from: this.currentNickname,
      body: message,
      timestamp: Date.now(),
    };
    console.log(`DEBUG: Created message with from='${newMessage.from}'`);

    chatState.messages.push(newMessage);
    await persistChatState(this.state, chatState);

    // Get the server instance to broadcast
    const server = (globalThis as any).serverInstance as CapnWebDurable;
    if (server) {
      await server.broadcastMessage(newMessage);
    }

    return { status: 'ok', echo: message };
  }

  async receiveMessages() {
    const chatState = await loadChatState(this.state);

    return {
      messages: chatState.messages.map(msg => ({
        from: msg.from,
        body: msg.body,
        timestamp: msg.timestamp,
      })),
    };
  }

  async whoami() {
    return { username: this.username };
  }

  async registerNick(nickname: string, password: string) {
    const chatState = await loadChatState(this.state);

    if (chatState.registeredNicks[nickname]) {
      return {
        status: 'error',
        message: 'Nickname already registered'
      };
    }

    chatState.registeredNicks[nickname] = password;
    chatState.nickOwners[nickname] = this.username;
    await persistChatState(this.state, chatState);

    // Update the current nickname for this session
    this.currentNickname = nickname;

    return {
      status: 'ok',
      message: `Nickname '${nickname}' registered successfully`
    };
  }

  async identifyNick(nickname: string, password: string) {
    const passwordSummary = typeof password === 'string' ? password.length : 'unknown';
    console.log(`DEBUG: identifyNick called with nickname='${nickname}', password length='${passwordSummary}', username='${this.username}'`);
    const chatState = await loadChatState(this.state);

    const storedPassword = chatState.registeredNicks[nickname];
    console.log(`DEBUG: Stored password for '${nickname}': ${storedPassword ? 'exists' : 'not found'}`);
    if (!storedPassword) {
      console.log(`DEBUG: Nickname '${nickname}' not registered`);
      return {
        status: 'error',
        message: 'Nickname not registered'
      };
    }

    if (storedPassword !== password) {
      console.log(`DEBUG: Password mismatch for '${nickname}' - stored: '${storedPassword}', provided: '${password}'`);
      return {
        status: 'error',
        message: 'Invalid password'
      };
    }

    const owner = chatState.nickOwners[nickname];
    console.log(`DEBUG: Owner of '${nickname}': '${owner}', current username: '${this.username}'`);
    if (owner && owner !== this.username) {
      console.log(`DEBUG: Ownership transfer for nickname '${nickname}' from '${owner}' to '${this.username}' after successful password check`);
    }

    // Record the latest owner and update the nickname for this session
    chatState.nickOwners[nickname] = this.username;
    console.log(`DEBUG: Updating currentNickname from '${this.currentNickname}' to '${nickname}'`);
    this.currentNickname = nickname;

    await persistChatState(this.state, chatState);

    console.log(`DEBUG: Identify successful for '${nickname}'`);
    const result = {
      status: 'ok',
      message: `Successfully identified as '${nickname}'`
    };
    console.log(`DEBUG: Returning result:`, result);
    return result;
  }

  async checkNick(nickname: string) {
    const chatState = await loadChatState(this.state);
    return {
      status: 'ok',
      registered: !!chatState.registeredNicks[nickname]
    };
  }

  async log(message: string) {
    console.log(`DEBUG: log method called with message: ${message}`);
    console.log(`CLIENT LOG [${this.username}]: ${message}`);
    return { status: 'ok' };
  }
}

export class CapnWebDurable extends RpcTarget {
  private clients: Set<ChatClientStub> = new Set();
  private sessions: Map<number, ChatSession> = new Map();

  constructor(private readonly state: DurableObjectStateWithStorage, private readonly env: Env) {
    super();
    // Store reference for sessions to access
    (globalThis as any).serverInstance = this;
  }

  async fetch(request: Request): Promise<Response> {
    // Handle WebSocket upgrade with capnweb
    if (request.headers.get('Upgrade') === 'websocket') {
      const pair = new WebSocketPair();
      const [serverSocket, clientSocket] = [pair[0], pair[1]];
      serverSocket.accept();

      console.log('Creating WebSocket RPC session');
      const clientStub = newWebSocketRpcSession<ChatClientStub>(serverSocket, this);
      console.log('Client stub created, registering client');
      this.registerClient(clientStub);
      console.log('Client registered, total clients:', this.clients.size);

      return new Response(null, {
        status: 101,
        webSocket: clientSocket,
      });
    }

    // Handle regular HTTP requests (for backward compatibility)
    return handleRpc(request, this.state);
  }

  // RPC methods that clients can call
  async auth(username: string, password: string) {
    const chatState = await loadChatState(this.state);

    // Accept any username with default password
    if (password !== 'default_password') {
      throw new Error('invalid credentials');
    }

    // Allocate session capability
    let sessionCapId = chatState.nextSessionCapId;
    while (chatState.sessionCaps[String(sessionCapId)]) {
      sessionCapId += 1;
    }
    chatState.nextSessionCapId = sessionCapId + 1;
    chatState.sessionCaps[String(sessionCapId)] = {
      username,
      displayName: username,
    };

    await persistChatState(this.state, chatState);

    return {
      session: {
        _type: 'capability',
        id: sessionCapId,
      },
      user: username,
    };
  }

  // This method will be called by clients to send messages
  async sendMessage(capabilityId: number, message: string) {
    console.log('Server sendMessage called with capabilityId:', capabilityId, 'message:', message);
    const chatState = await loadChatState(this.state);

    // Find the user by capability ID
    const sessionInfo = chatState.sessionCaps[String(capabilityId)];
    if (!sessionInfo) {
      throw new Error('unknown session capability');
    }

    const from = sessionInfo.displayName ?? sessionInfo.username;

    const newMessage = {
      from,
      body: message,
      timestamp: Date.now(),
    };

    chatState.messages.push(newMessage);
    await persistChatState(this.state, chatState);

    // Broadcast to all connected clients
    for (const clientStub of Array.from(this.clients)) {
      try {
        // Call the receiveMessage method on each client
        await clientStub.receiveMessage(newMessage);
      } catch (error) {
        console.error('Failed to send message to client:', error);
        this.clients.delete(clientStub);
      }
    }

    return { status: 'ok', echo: message };
  }

  async receiveMessages(capabilityId: number) {
    console.log('Server receiveMessages called with capabilityId:', capabilityId);
    const chatState = await loadChatState(this.state);
    console.log('Loaded chat state with', chatState.messages.length, 'messages');

    // Verify the session exists
    const sessionInfo = chatState.sessionCaps[String(capabilityId)];
    if (!sessionInfo) {
      console.log('Session not found for capabilityId:', capabilityId);
      throw new Error('unknown session capability');
    }

    console.log('Returning', chatState.messages.length, 'messages');
    return {
      messages: chatState.messages.map(msg => ({
        from: msg.from,
        body: msg.body,
        timestamp: msg.timestamp,
      })),
    };
  }

  async whoami(capabilityId: number) {
    console.log('Server whoami called with capabilityId:', capabilityId);
    const chatState = await loadChatState(this.state);

    const sessionInfo = chatState.sessionCaps[String(capabilityId)];
    if (!sessionInfo) {
      throw new Error('unknown session capability');
    }

    const username = sessionInfo.displayName ?? sessionInfo.username;
    return { username };
  }

  async registerNick(capabilityId: number, nickname: string, password: string) {
    if (typeof nickname !== 'string' || typeof password !== 'string') {
      throw new TypeError('`registerNick` expects <capabilityId>, <nickname>, <password>');
    }

    const chatState = await loadChatState(this.state);
    const sessionInfo = chatState.sessionCaps[String(capabilityId)];
    if (!sessionInfo) {
      throw new Error('unknown session capability');
    }

    if (chatState.registeredNicks[nickname]) {
      return {
        status: 'error',
        message: 'Nickname already registered',
      };
    }

    chatState.registeredNicks[nickname] = password;
    chatState.nickOwners[nickname] = sessionInfo.username;
    sessionInfo.displayName = nickname;
    chatState.sessionCaps[String(capabilityId)] = sessionInfo;

    await persistChatState(this.state, chatState);

    return {
      status: 'ok',
      message: `Nickname '${nickname}' registered successfully`,
    };
  }

  async identifyNick(capabilityId: number, nickname: string, password: string) {
    if (typeof nickname !== 'string' || typeof password !== 'string') {
      throw new TypeError('`identifyNick` expects <capabilityId>, <nickname>, <password>');
    }

    const chatState = await loadChatState(this.state);
    const sessionInfo = chatState.sessionCaps[String(capabilityId)];
    if (!sessionInfo) {
      throw new Error('unknown session capability');
    }

    const storedPassword = chatState.registeredNicks[nickname];
    if (!storedPassword) {
      return {
        status: 'error',
        message: 'Nickname not registered',
      };
    }

    if (storedPassword !== password) {
      return {
        status: 'error',
        message: 'Invalid password',
      };
    }

    const owner = chatState.nickOwners[nickname];
    if (owner && owner !== sessionInfo.username) {
      console.log(`DEBUG: Ownership transfer for nickname '${nickname}' from '${owner}' to '${sessionInfo.username}' after successful password check`);
    }

    chatState.nickOwners[nickname] = sessionInfo.username;
    sessionInfo.displayName = nickname;
    chatState.sessionCaps[String(capabilityId)] = sessionInfo;

    await persistChatState(this.state, chatState);

    return {
      status: 'ok',
      message: `Successfully identified as '${nickname}'`,
    };
  }

  async checkNick(capabilityId: number, nickname: string) {
    if (typeof nickname !== 'string') {
      throw new TypeError('`checkNick` expects <capabilityId>, <nickname>');
    }

    const chatState = await loadChatState(this.state);
    const sessionInfo = chatState.sessionCaps[String(capabilityId)];
    if (!sessionInfo) {
      throw new Error('unknown session capability');
    }

    return {
      status: 'ok',
      registered: !!chatState.registeredNicks[nickname],
    };
  }

  async broadcastMessage(message: { from: string; body: string; timestamp: number }) {
    console.log(`Broadcasting message to ${this.clients.size} clients:`, message);
    // Broadcast to all connected clients
    for (const clientStub of Array.from(this.clients)) {
      try {
        console.log('Calling receiveMessage on client stub');
        // Call the receiveMessage method on each client
        // The clientStub should be the client's RPC target
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

  async log(capabilityId: number, message: string) {
    console.log(`DEBUG: log method called with capabilityId: ${capabilityId}, message: ${message}`);
    
    // Look up session in chat state instead of sessions map
    const chatState = await loadChatState(this.state);
    const sessionCap = chatState.sessionCaps[String(capabilityId)];
    if (sessionCap) {
      const label = sessionCap.displayName ?? sessionCap.username;
      console.log(`CLIENT LOG [${label}]: ${message}`);
    } else {
      console.log(`CLIENT LOG [unknown session ${capabilityId}]: ${message}`);
    }
    return { status: 'ok' };
  }
}

async function handleRpc(request: Request, state: DurableObjectStateWithStorage): Promise<Response> {
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
    const responseBody = await processRpcBatch(payload, state);

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

type SessionInfo = {
  username: string;
  displayName?: string;
};

type ChatState = {
  credentials: Record<string, string>;
  messages: Array<{ from: string; body: string; timestamp: number }>;
  nextSessionCapId: number;
  sessionCaps: Record<string, SessionInfo>;
  registeredNicks: Record<string, string>; // nickname -> password
  nickOwners: Record<string, string>; // nickname -> username
};

const DEFAULT_CHAT_STATE: ChatState = {
  credentials: {},
  messages: [],
  nextSessionCapId: SESSION_CAPABILITY_START,
  sessionCaps: {},
  registeredNicks: {},
  nickOwners: {},
};

function cloneDefaultChatState(): ChatState {
  return {
    credentials: { ...DEFAULT_CHAT_STATE.credentials },
    messages: [...DEFAULT_CHAT_STATE.messages],
    nextSessionCapId: DEFAULT_CHAT_STATE.nextSessionCapId,
    sessionCaps: { ...DEFAULT_CHAT_STATE.sessionCaps },
    registeredNicks: { ...DEFAULT_CHAT_STATE.registeredNicks },
    nickOwners: { ...DEFAULT_CHAT_STATE.nickOwners },
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
            ...(typeof displayName === 'string' ? { displayName } : {}),
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

  return {
    credentials,
    messages,
    nextSessionCapId,
    sessionCaps,
    registeredNicks,
    nickOwners,
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
