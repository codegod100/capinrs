import {
  ChangeEvent,
  FormEvent,
  KeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { newWebSocketRpcSession, RpcTarget } from 'capnweb';
import { LoginForm } from './components/LoginForm';

const DEFAULT_BACKEND = 'wss://capinrs-server.veronika-m-winters.workers.dev';
const STATUS_HELP = 'Type /help for commands | Press Esc to cancel password input';
const MAX_MESSAGES = 200;
const MAX_HISTORY = 50;
const TOKEN_STORAGE_KEY = 'capinrs:nickserv-token';
const LAST_URL_STORAGE_KEY = 'capinrs:last-server-url';
const DEFAULT_VISIBLE_LINES = 15;

type Message = {
  from: string;
  body: string;
  timestamp: number;
};

type Session = {
  capabilityId: number;
  username: string;
  nickname: string;
  serverUrl: string;
};

type PasswordPromptState = {
  command: 'identify' | 'register';
  nickname: string;
};

type RpcSessionHandles = {
  remoteMain: any;
  client: ChatClient;
};

type StoredTokenRecord = {
  token: string;
  nickname?: string;
  username?: string;
  issuedAt: number;
  lastUsed?: number;
};

class ChatClient extends RpcTarget {
  public onMessage?: (message: Message) => void;

  receiveMessage(message: Message) {
    if (this.onMessage) {
      this.onMessage(normalizeMessage(message));
    }
  }
}

async function createRpcSession(url: string): Promise<RpcSessionHandles> {
  const client = new ChatClient();
  const remoteMain = await newWebSocketRpcSession(url, client);
  return { remoteMain, client };
}

function generateRandomNickname(): string {
  const adjectives = [
    'Happy',
    'Clever',
    'Swift',
    'Bright',
    'Calm',
    'Bold',
    'Wise',
    'Kind',
    'Cool',
    'Sharp',
  ];
  const nouns = [
    'Cat',
    'Dog',
    'Bird',
    'Fish',
    'Bear',
    'Wolf',
    'Fox',
    'Lion',
    'Tiger',
    'Eagle',
  ];
  const adj = adjectives[Math.floor(Math.random() * adjectives.length)];
  const noun = nouns[Math.floor(Math.random() * nouns.length)];
  const num = Math.floor(Math.random() * 900) + 100;
  return `${adj}${noun}${num}`;
}

function formatStatus(nickname: string, serverUrl: string, detail?: string): string {
  if (detail && detail.trim().length > 0) {
    return `Server: ${serverUrl} | Nick: ${nickname} | ${detail}`;
  }
  return `Server: ${serverUrl} | Nick: ${nickname}`;
}

function normalizeEndpoint(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) {
    return DEFAULT_BACKEND;
  }
  if (/^wss?:\/\//i.test(trimmed)) {
    return trimmed;
  }
  if (/^https?:\/\//i.test(trimmed)) {
    return trimmed.replace(/^http/i, 'ws');
  }
  if (trimmed.startsWith('localhost') || trimmed.startsWith('127.') || trimmed.startsWith('192.168.')) {
    return `ws://${trimmed}`;
  }
  return `wss://${trimmed}`;
}

function normalizeMessage(raw: any): Message {
  return {
    from: typeof raw?.from === 'string' ? raw.from : 'Unknown',
    body: typeof raw?.body === 'string' ? raw.body : '',
    timestamp: typeof raw?.timestamp === 'number' ? raw.timestamp : Date.now(),
  };
}

function createSystemMessage(body: string): Message {
  return {
    from: 'System',
    body,
    timestamp: Date.now(),
  };
}

function limitMessages(messages: Message[]): Message[] {
  if (messages.length <= MAX_MESSAGES) {
    return messages;
  }
  return messages.slice(messages.length - MAX_MESSAGES);
}

function formatTimestamp(timestamp: number): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) {
    return '';
  }
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

function supportsStorage(): boolean {
  try {
    return typeof window !== 'undefined' && typeof window.localStorage !== 'undefined';
  } catch {
    return false;
  }
}

function readStoredToken(): StoredTokenRecord | null {
  if (!supportsStorage()) {
    return null;
  }
  try {
    const raw = window.localStorage.getItem(TOKEN_STORAGE_KEY);
    if (!raw) {
      return null;
    }
    const parsed = JSON.parse(raw) as StoredTokenRecord;
    if (!parsed || typeof parsed.token !== 'string' || parsed.token.length === 0) {
      return null;
    }
    return parsed;
  } catch {
    return null;
  }
}

function writeStoredToken(record: StoredTokenRecord) {
  if (!supportsStorage()) {
    return;
  }
  try {
    window.localStorage.setItem(TOKEN_STORAGE_KEY, JSON.stringify(record));
  } catch {
    // Ignore storage errors (e.g., quota exceeded, privacy mode)
  }
}

function clearStoredToken() {
  if (!supportsStorage()) {
    return;
  }
  try {
    window.localStorage.removeItem(TOKEN_STORAGE_KEY);
  } catch {
    // Ignore
  }
}

function generateNickToken(): string {
  if (typeof window !== 'undefined' && window.crypto?.getRandomValues) {
    const bytes = new Uint8Array(16);
    window.crypto.getRandomValues(bytes);
    return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

function App() {
  const [phase, setPhase] = useState<'login' | 'connecting' | 'chat'>('login');
  const [connectError, setConnectError] = useState<string | null>(null);
  const [session, setSession] = useState<Session | null>(null);
  const [handles, setHandles] = useState<RpcSessionHandles | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [status, setStatus] = useState('');
  const [isStatusError, setIsStatusError] = useState(false);
  const [inputValue, setInputValue] = useState('');
  const [passwordValue, setPasswordValue] = useState('');
  const [passwordPrompt, setPasswordPrompt] = useState<PasswordPromptState | null>(null);
  const [history, setHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(0);
  const [savedTokenInfo, setSavedTokenInfo] = useState<StoredTokenRecord | null>(null);
  const [isRedeemingToken, setIsRedeemingToken] = useState(false);
  const [rememberedUrl, setRememberedUrl] = useState(() => readLastUrl() ?? DEFAULT_BACKEND);
  const [visibleLines, setVisibleLines] = useState(DEFAULT_VISIBLE_LINES);

  const messagesRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const shouldAutoscrollRef = useRef(true);

  const messageCount = messages.length;

  const displayMessages = useMemo(() => messages, [messages]);

  const connecting = phase === 'connecting';

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }

    const handleResize = () => {
      const height = window.innerHeight;
      const header = 220;
      const footer = 250;
      const available = Math.max(height - header - footer, 140);
      const lineHeight = 28;
      const lines = Math.max(Math.floor(available / lineHeight), 6);
      setVisibleLines(lines);
    };

    handleResize();
    window.addEventListener('resize', handleResize);

    const storedToken = readStoredToken();
    if (storedToken) {
      setSavedTokenInfo(storedToken);
    }
    const storedUrl = readLastUrl();
    if (storedUrl) {
      setRememberedUrl(storedUrl);
    }

    return () => {
      window.removeEventListener('resize', handleResize);
    };
  }, []);

  useEffect(() => {
    if (phase === 'chat' && inputRef.current) {
      inputRef.current.focus();
    }
  }, [phase]);

  useEffect(() => {
    if (!handles || !session) {
      return;
    }

    const { client } = handles;
    const handler = (message: Message) => {
      setMessages((prev) => limitMessages([...prev, normalizeMessage(message)]));
    };

    client.onMessage = handler;

    return () => {
      client.onMessage = undefined;
    };
  }, [handles, session]);

  useEffect(() => {
    if (shouldAutoscrollRef.current && messagesRef.current) {
      messagesRef.current.scrollTop = messagesRef.current.scrollHeight;
    }
  }, [messages]);

  const handleMessagesScroll = useCallback(() => {
    const node = messagesRef.current;
    if (!node) {
      return;
    }

    const { scrollTop, scrollHeight, clientHeight } = node;
    const atBottom = scrollHeight - (scrollTop + clientHeight) < 16;
    shouldAutoscrollRef.current = atBottom;
  }, []);

  const safeLog = useCallback(
    async (text: string) => {
      if (!handles || !session) {
        return;
      }
      try {
        await handles.remoteMain.log(session.capabilityId, text);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setMessages((prev) =>
          limitMessages([
            ...prev,
            createSystemMessage(`Log RPC failed: ${message}`),
          ]),
        );
      }
    },
    [handles, session],
  );

  const createNickToken = useCallback(
    async ({
      remoteMain,
      capabilityId,
      nickname,
      username,
      serverUrl,
    }: {
      remoteMain: any;
      capabilityId: number;
      nickname: string;
      username: string;
      serverUrl: string;
    }) => {
      if (typeof remoteMain?.storeNickToken !== 'function') {
        setMessages((prev) =>
          limitMessages([
            ...prev,
            createSystemMessage(
              'Server does not support NickServ tokens yet. Update the backend to enable token storage.',
            ),
          ]),
        );
        setStatus(formatStatus(nickname, serverUrl, `${STATUS_HELP}`));
        setIsStatusError(false);
        return;
      }
      const token = generateNickToken();
      try {
        await remoteMain.storeNickToken(capabilityId, token);
        const record: StoredTokenRecord = {
          token,
          nickname,
          username,
          issuedAt: Date.now(),
        };
        writeStoredToken(record);
        setSavedTokenInfo(record);
        setMessages((prev) =>
          limitMessages([
            ...prev,
            createSystemMessage(
              `NickServ token stored locally for '${nickname}'. You can reuse it from the login screen.`,
            ),
          ]),
        );
        await safeLog(`NickServ token stored for '${nickname}'`);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setMessages((prev) =>
          limitMessages([
            ...prev,
            createSystemMessage(`Failed to store NickServ token: ${message}`),
          ]),
        );
        setStatus(formatStatus(nickname, serverUrl, `Token storage failed: ${message}`));
        setIsStatusError(true);
      }
    },
    [safeLog, setIsStatusError, setStatus],
  );

  const pushHistory = useCallback((command: string) => {
    setHistory((prev) => {
      const trimmed = command.trim();
      if (!trimmed || prev[prev.length - 1] === trimmed) {
        setHistoryIndex(prev.length);
        return prev;
      }
      const next = prev.length >= MAX_HISTORY ? [...prev.slice(1), trimmed] : [...prev, trimmed];
      setHistoryIndex(next.length);
      return next;
    });
  }, []);

  const runNickCommand = useCallback(
    async (mode: 'identify' | 'register', nickname: string, password: string) => {
      if (!handles || !session) {
        return;
      }

      const { remoteMain } = handles;
      const capabilityId = session.capabilityId;

      try {
        if (mode === 'identify') {
          await safeLog(`NickServ identify requested for '${nickname}'`);
          const result = await remoteMain.identifyNick(capabilityId, nickname, password);
          const statusValue = typeof result?.status === 'string' ? result.status : '';
          const messageValue = typeof result?.message === 'string' ? result.message : 'NickServ identify succeeded.';
          if (statusValue === 'ok') {
            setSession((prev) => (prev ? { ...prev, nickname } : prev));
            setStatus(formatStatus(nickname, session.serverUrl, `${messageValue} | ${STATUS_HELP}`));
            setIsStatusError(false);
            setMessages((prev) =>
              limitMessages([
                ...prev,
                createSystemMessage(messageValue),
              ]),
            );
            await safeLog(`NickServ identify succeeded; nickname is now '${nickname}'`);
            await createNickToken({
              remoteMain,
              capabilityId,
              nickname,
              username: session.username,
              serverUrl: session.serverUrl,
            });
          } else {
            throw new Error(messageValue);
          }
        } else {
          await safeLog(`NickServ register requested for '${nickname}'`);
          const result = await remoteMain.registerNick(capabilityId, nickname, password);
          const statusValue = typeof result?.status === 'string' ? result.status : '';
          const messageValue = typeof result?.message === 'string' ? result.message : `Nickname '${nickname}' registered successfully.`;
          if (statusValue === 'ok') {
            setSession((prev) => (prev ? { ...prev, nickname } : prev));
            setStatus(formatStatus(nickname, session.serverUrl, `${messageValue} | ${STATUS_HELP}`));
            setIsStatusError(false);
            setMessages((prev) =>
              limitMessages([
                ...prev,
                createSystemMessage(messageValue),
              ]),
            );
            await safeLog(`NickServ register succeeded; nickname is now '${nickname}'`);
            await createNickToken({
              remoteMain,
              capabilityId,
              nickname,
              username: session.username,
              serverUrl: session.serverUrl,
            });
          } else {
            throw new Error(messageValue);
          }
        }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        const action = mode === 'identify' ? 'NickServ identify failed' : 'NickServ register failed';
        setStatus(formatStatus(session.nickname, session.serverUrl, `${action}: ${message}`));
        setIsStatusError(true);
        setMessages((prev) =>
          limitMessages([
            ...prev,
            createSystemMessage(`${action}: ${message}`),
          ]),
        );
      }
    },
    [createNickToken, handles, safeLog, session],
  );

  const handleConnect = useCallback(
    async ({
      url,
      nickname,
      nicknamePassword,
    }: {
      url: string;
      nickname?: string;
      nicknamePassword?: string;
    }) => {
      const normalizedUrl = normalizeEndpoint(url);
      setPhase('connecting');
      setConnectError(null);
      setMessages([]);
      setHistory([]);
      setHistoryIndex(0);
      setPasswordPrompt(null);
      setPasswordValue('');
      setStatus('Connecting…');
      setIsStatusError(false);

      try {
        const { remoteMain, client } = await createRpcSession(normalizedUrl);
        const chosenNickname = nickname && nickname.trim().length > 0 ? nickname : generateRandomNickname();
        const authResult = await remoteMain.auth(chosenNickname, '');
        const capabilityRaw = authResult?.session?.id ?? authResult?.sessionId;
        const capabilityId = Number(capabilityRaw);

        if (!Number.isFinite(capabilityId)) {
          throw new Error('Authentication response missing session capability');
        }

        const resolvedNickname =
          typeof authResult?.user === 'string' && authResult.user.length > 0
            ? authResult.user
            : chosenNickname;

        const nextSession: Session = {
          capabilityId,
          username: chosenNickname,
          nickname: resolvedNickname,
          serverUrl: normalizedUrl,
        };

        setHandles({ remoteMain, client });
        setSession(nextSession);
        writeLastUrl(normalizedUrl);
        setRememberedUrl(normalizedUrl);

        await safeLog('Client connected successfully via Pages UI');

        const messagesResult = await remoteMain.receiveMessages(capabilityId);
        const rawMessages: any[] = Array.isArray(messagesResult?.messages)
          ? messagesResult.messages
          : [];
        const normalizedMessages = rawMessages.map(normalizeMessage);
        const limited = limitMessages(normalizedMessages);

        setMessages(() =>
          limitMessages([
            ...limited,
            createSystemMessage(`Welcome, ${resolvedNickname}! Type /help for available commands.`),
          ]),
        );

        setStatus(
          formatStatus(
            resolvedNickname,
            normalizedUrl,
            `Loaded ${Math.min(limited.length, MAX_MESSAGES)} recent messages (of ${rawMessages.length} total) | ${STATUS_HELP}`,
          ),
        );
        setIsStatusError(false);
        shouldAutoscrollRef.current = true;
        setPhase('chat');

        if (nickname && nicknamePassword) {
          try {
            const checkResult = await remoteMain.checkNick(capabilityId, nickname);
            const registered = Boolean(checkResult?.registered);
            if (!registered) {
              const message = `Nickname '${nickname}' is not registered. Use /nickserv register <nick>.`;
              setStatus(formatStatus(resolvedNickname, normalizedUrl, `${message} | ${STATUS_HELP}`));
              setIsStatusError(true);
              setMessages((prev) =>
                limitMessages([
                  ...prev,
                  createSystemMessage(`NickServ identify aborted: ${message}`),
                ]),
              );
            } else {
              try {
                const identifyResult = await remoteMain.identifyNick(
                  capabilityId,
                  nickname,
                  nicknamePassword,
                );
                const statusValue = typeof identifyResult?.status === 'string' ? identifyResult.status : '';
                const messageValue =
                  typeof identifyResult?.message === 'string'
                    ? identifyResult.message
                    : `Successfully identified as '${nickname}'`;

                if (statusValue === 'ok') {
                  setSession((prev) => ({ ...(prev ?? nextSession), nickname }));
                  setStatus(
                    formatStatus(
                      nickname,
                      normalizedUrl,
                      `${messageValue} | ${STATUS_HELP}`,
                    ),
                  );
                  setIsStatusError(false);
                  setMessages((prev) =>
                    limitMessages([
                      ...prev,
                      createSystemMessage(messageValue),
                    ]),
                  );

                  await createNickToken({
                    remoteMain,
                    capabilityId,
                    nickname,
                    username: nextSession.username,
                    serverUrl: normalizedUrl,
                  });
                } else {
                  throw new Error(messageValue);
                }
              } catch (error) {
                const message = error instanceof Error ? error.message : String(error);
                setStatus(
                  formatStatus(
                    resolvedNickname,
                    normalizedUrl,
                    `NickServ identify failed: ${message}`,
                  ),
                );
                setIsStatusError(true);
                setMessages((prev) =>
                  limitMessages([
                    ...prev,
                    createSystemMessage(`NickServ identify failed: ${message}`),
                  ]),
                );
              }
            }
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            setStatus(formatStatus(resolvedNickname, normalizedUrl, `Failed to verify nickname '${nickname}': ${message}`));
            setIsStatusError(true);
            setMessages((prev) =>
              limitMessages([
                ...prev,
                createSystemMessage(`Failed to verify nickname '${nickname}': ${message}`),
              ]),
            );
          }
        }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setPhase('login');
        setConnectError(message);
        setHandles(null);
        setSession(null);
        setStatus('');
        setIsStatusError(false);
      }
    },
    [runNickCommand, safeLog],
  );

  const redeemTokenInternal = useCallback(
    async ({ url, stored }: { url: string; stored: StoredTokenRecord }) => {
      
      const normalizedUrl = normalizeEndpoint(url);
      setIsRedeemingToken(true);
      setPhase('connecting');
      setConnectError(null);
      setMessages([]);
      setHistory([]);
      setHistoryIndex(0);
      setPasswordPrompt(null);
      setPasswordValue('');
      setStatus('Redeeming token…');
      setIsStatusError(false);

      try {
        const { remoteMain, client } = await createRpcSession(normalizedUrl);
        if (typeof remoteMain?.redeemNickToken !== 'function') {
          throw new Error('This server build does not support token redemption yet.');
        }
        const redeemResult = await remoteMain.redeemNickToken(stored.token);
        if (redeemResult?.status !== 'ok') {
          const message =
            typeof redeemResult?.message === 'string'
              ? redeemResult.message
              : 'Token redemption failed';
          throw new Error(message);
        }

        const capabilityRaw = redeemResult?.session?.id ?? redeemResult?.sessionId;
        const capabilityId = Number(capabilityRaw);
        if (!Number.isFinite(capabilityId)) {
          throw new Error('Redeem response missing session capability');
        }

        const resolvedUser =
          typeof redeemResult?.user === 'string' && redeemResult.user.length > 0
            ? redeemResult.user
            : stored.username ?? 'unknown';
        const resolvedNickname =
          typeof redeemResult?.nickname === 'string' && redeemResult.nickname.length > 0
            ? redeemResult.nickname
            : stored.nickname ?? resolvedUser;

        const nextSession: Session = {
          capabilityId,
          username: resolvedUser,
          nickname: resolvedNickname,
          serverUrl: normalizedUrl,
        };

        setHandles({ remoteMain, client });
        setSession(nextSession);
        writeLastUrl(normalizedUrl);
        setRememberedUrl(normalizedUrl);

        const messagesResult = await remoteMain.receiveMessages(capabilityId);
        const rawMessages: any[] = Array.isArray(messagesResult?.messages)
          ? messagesResult.messages
          : [];
        const normalizedMessages = rawMessages.map(normalizeMessage);
        const limited = limitMessages(normalizedMessages);

        setMessages(() =>
          limitMessages([
            ...limited,
            createSystemMessage(`Welcome back, ${resolvedNickname}! Signed in via saved token.`),
          ]),
        );

        setStatus(
          formatStatus(
            resolvedNickname,
            normalizedUrl,
            `Signed in with saved token | ${STATUS_HELP}`,
          ),
        );
        setIsStatusError(false);
        shouldAutoscrollRef.current = true;
        setPhase('chat');

        const updatedRecord: StoredTokenRecord = {
          token: stored.token,
          username: resolvedUser,
          nickname: resolvedNickname,
          issuedAt: stored.issuedAt,
          lastUsed: Date.now(),
        };
        writeStoredToken(updatedRecord);
        setSavedTokenInfo(updatedRecord);

        try {
          await remoteMain.log(capabilityId, 'Client connected via saved token');
        } catch {
          // Ignore log failures
        }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setPhase('login');
        setConnectError(message);
        setStatus('');
        setIsStatusError(false);
        setHandles(null);
        setSession(null);
        setMessages([]);
        setHistory([]);
        setHistoryIndex(0);

        const lowered = message.toLowerCase();
        if (lowered.includes('token') && lowered.includes('not')) {
          clearStoredToken();
          setSavedTokenInfo(null);
        }
      } finally {
        setIsRedeemingToken(false);
      }
    },
    [],
  );

  const handleRedeemToken = useCallback(
    async ({ url }: { url: string }) => {
      const stored = readStoredToken();
      if (!stored) {
        setConnectError('No saved token is available to redeem.');
        return;
      }
      await redeemTokenInternal({ url, stored });
    },
    [redeemTokenInternal],
  );

  const handleClearToken = useCallback(() => {
    clearStoredToken();
    setSavedTokenInfo(null);
  }, []);

  useEffect(() => {
    if (
      phase === 'login' &&
      savedTokenInfo &&
      rememberedUrl &&
      !isRedeemingToken &&
      !handles
    ) {
      redeemTokenInternal({ url: rememberedUrl, stored: savedTokenInfo }).catch((error: unknown) => {
        console.error('Automatic token redemption failed:', error);
        setPhase('login');
      });
    }
  }, [phase, rememberedUrl, redeemTokenInternal, savedTokenInfo, isRedeemingToken, handles]);

  const handleLogout = useCallback(() => {
    clearStoredToken();
    setSavedTokenInfo(null);
    setHandles(null);
    setSession(null);
    setMessages([]);
    setHistory([]);
    setHistoryIndex(0);
   setPasswordPrompt(null);
   setPasswordValue('');
    setPhase('login');
    setStatus('');
    setIsStatusError(false);
  }, []);

  const handleCommand = useCallback(
    async (input: string) => {
      if (!handles || !session) {
        return;
      }

      const trimmed = input.trim();
      if (!trimmed) {
        return;
      }

      await safeLog(`Command received: '${trimmed}'`);

      const { remoteMain } = handles;

      if (!trimmed.startsWith('/')) {
        try {
          await remoteMain.sendMessage(session.capabilityId, trimmed);
          setStatus(formatStatus(session.nickname, session.serverUrl, STATUS_HELP));
          setIsStatusError(false);
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          setStatus(formatStatus(session.nickname, session.serverUrl, `Failed to send message: ${message}`));
          setIsStatusError(true);
        }
        return;
      }

      const parts = trimmed.split(/\s+/);
      const command = parts[0].toLowerCase();

      switch (command) {
        case '/quit':
        case '/exit': {
          setStatus(formatStatus(session.nickname, session.serverUrl, `Close the browser tab to exit | ${STATUS_HELP}`));
          setIsStatusError(false);
          setMessages((prev) =>
            limitMessages([
              ...prev,
              createSystemMessage('Browser clients cannot exit the session automatically. Close this tab to quit.'),
            ]),
          );
          break;
        }
        case '/help': {
          setMessages((prev) =>
            limitMessages([
              ...prev,
              createSystemMessage(`Available Commands:
  /help                  Show this help
  /whoami                Show current session
  /receive               Fetch and display messages
/nickserv identify <nick>  Identify with a protected nickname
/nickserv register <nick>  Register a new nickname
  /token info            Show saved token details (browser only)
  /token clear           Remove the saved token from this browser
  /quit                  Exit the client

Messages without a leading slash are broadcast to the chat.`),
            ]),
          );
          break;
        }
        case '/whoami': {
          try {
            const result = await remoteMain.whoami(session.capabilityId);
            const username = typeof result?.username === 'string' ? result.username : JSON.stringify(result);
            setMessages((prev) =>
              limitMessages([
                ...prev,
                createSystemMessage(`You are ${username}`),
              ]),
            );
            setStatus(formatStatus(session.nickname, session.serverUrl, `Authenticated as ${username} | ${STATUS_HELP}`));
            setIsStatusError(false);
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            setStatus(formatStatus(session.nickname, session.serverUrl, `Whoami failed: ${message}`));
            setIsStatusError(true);
          }
          break;
        }
        case '/receive': {
          try {
            const result = await remoteMain.receiveMessages(session.capabilityId);
            const rawMessages: any[] = Array.isArray(result?.messages) ? result.messages : [];
            const normalizedMessages = rawMessages.map(normalizeMessage);
            setMessages((prev) => limitMessages([...prev, ...normalizedMessages]));
            setStatus(formatStatus(session.nickname, session.serverUrl, `Fetched ${normalizedMessages.length} messages | ${STATUS_HELP}`));
            setIsStatusError(false);
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            setStatus(formatStatus(session.nickname, session.serverUrl, `Failed to receive messages: ${message}`));
            setIsStatusError(true);
          }
          break;
        }
        case '/nickserv': {
          if (parts.length < 2) {
            setMessages((prev) =>
              limitMessages([
                ...prev,
                createSystemMessage(`NickServ Commands:
/nickserv identify <nick>  Identify with a protected nickname
/nickserv register <nick>  Register a new nickname`),
              ]),
            );
            break;
          }

          const subcommand = parts[1].toLowerCase();
          if (subcommand === 'identify') {
            if (parts.length < 3) {
              setMessages((prev) =>
                limitMessages([
                  ...prev,
                  createSystemMessage(`Usage: /nickserv identify <nick>
You will be prompted for the nickname password.`),
                ]),
              );
              break;
            }
            const nick = parts[2];
            try {
              const checkResult = await remoteMain.checkNick(session.capabilityId, nick);
              const registered = Boolean(checkResult?.registered);
              if (!registered) {
                const message = `Nickname '${nick}' is not registered. Use /nickserv register <nick>.`;
                setMessages((prev) =>
                  limitMessages([
                    ...prev,
                    createSystemMessage(message),
                  ]),
                );
                setStatus(formatStatus(session.nickname, session.serverUrl, `${message} | ${STATUS_HELP}`));
                setIsStatusError(true);
                break;
              }
              setPasswordPrompt({ command: 'identify', nickname: nick });
              setPasswordValue('');
              setStatus(formatStatus(session.nickname, session.serverUrl, `Enter password for nickname '${nick}' and press Enter | ${STATUS_HELP}`));
              setIsStatusError(false);
              setMessages((prev) =>
                limitMessages([
                  ...prev,
                  createSystemMessage(`Please enter password for nickname '${nick}' in the input area below.`),
                ]),
              );
            } catch (error) {
              const message = error instanceof Error ? error.message : String(error);
              setStatus(formatStatus(session.nickname, session.serverUrl, `Failed to verify nickname '${nick}': ${message}`));
              setIsStatusError(true);
              setMessages((prev) =>
                limitMessages([
                  ...prev,
                  createSystemMessage(`Failed to verify nickname '${nick}': ${message}`),
                ]),
              );
            }
          } else if (subcommand === 'register') {
            if (parts.length < 3) {
              setMessages((prev) =>
                limitMessages([
                  ...prev,
                  createSystemMessage(`Usage: /nickserv register <nick>
You will be prompted for a password to protect your nickname.`),
                ]),
              );
              break;
            }
            const nick = parts[2];
            setPasswordPrompt({ command: 'register', nickname: nick });
            setPasswordValue('');
            setStatus(formatStatus(session.nickname, session.serverUrl, `Enter password for new nickname '${nick}' and press Enter | ${STATUS_HELP}`));
            setIsStatusError(false);
            setMessages((prev) =>
              limitMessages([
                ...prev,
                createSystemMessage(`Please enter password for new nickname '${nick}' in the input area below.`),
              ]),
            );
          } else {
            setMessages((prev) =>
              limitMessages([
                ...prev,
                createSystemMessage("Unknown nickserv command. Use 'identify' or 'register'."),
              ]),
            );
          }
          break;
        }
        case '/token': {
          const action = (parts[1] ?? '').toLowerCase();
          if (!action || action === 'help') {
            setMessages((prev) =>
              limitMessages([
                ...prev,
                createSystemMessage(`Token Commands:
/token info             Show saved token details (nickname, last used)
/token clear            Remove the saved token from this browser`),
              ]),
            );
            break;
          }

          if (action === 'info') {
            const stored = readStoredToken();
            if (!stored) {
              setMessages((prev) =>
                limitMessages([
                  ...prev,
                  createSystemMessage('No saved token is stored in this browser.'),
                ]),
              );
            } else {
              const nickname = stored.nickname ?? stored.username ?? 'unknown user';
              const issued = new Date(stored.issuedAt).toLocaleString();
              const lastUsed = stored.lastUsed
                ? new Date(stored.lastUsed).toLocaleString()
                : 'never used';
              setMessages((prev) =>
                limitMessages([
                  ...prev,
                  createSystemMessage(
                    `Saved token for ${nickname}. Issued ${issued}. Last used ${lastUsed}. Redeem from the login screen.`,
                  ),
                ]),
              );
            }
            break;
          }

          if (action === 'clear') {
            clearStoredToken();
            setSavedTokenInfo(null);
            setMessages((prev) =>
              limitMessages([
                ...prev,
                createSystemMessage('Saved token removed from this browser.'),
              ]),
            );
            setStatus(formatStatus(session.nickname, session.serverUrl, STATUS_HELP));
            setIsStatusError(false);
            break;
          }

          setMessages((prev) =>
            limitMessages([
              ...prev,
              createSystemMessage(
                `Unknown token command \`${action}\`. Use /token help for a list of subcommands.`,
              ),
            ]),
          );
          break;
        }
        default: {
          setMessages((prev) =>
            limitMessages([
              ...prev,
              createSystemMessage(`Unknown command \`${command}\`. Type /help for a list of commands.`),
            ]),
          );
          break;
        }
      }
    },
    [handles, safeLog, session],
  );

  const handlePasswordSubmit = useCallback(async () => {
    if (!passwordPrompt) {
      return;
    }
    const password = passwordValue.trim();
    if (!password) {
      if (session) {
        setStatus(formatStatus(session.nickname, session.serverUrl, 'Password cannot be empty | ' + STATUS_HELP));
        setIsStatusError(true);
      }
      return;
    }
    const { command, nickname } = passwordPrompt;
    setPasswordPrompt(null);
    setPasswordValue('');
    await runNickCommand(command, nickname, password);
    if (inputRef.current) {
      inputRef.current.focus();
    }
  }, [passwordPrompt, passwordValue, runNickCommand, session]);

  const handleInputSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!session) {
      return;
    }
    if (passwordPrompt) {
      await handlePasswordSubmit();
      return;
    }
    const trimmed = inputValue.trim();
    if (!trimmed) {
      setInputValue('');
      return;
    }
    pushHistory(trimmed);
    setInputValue('');
    await handleCommand(trimmed);
  };

  const handleInputChange = (event: ChangeEvent<HTMLInputElement>) => {
    if (passwordPrompt) {
      setPasswordValue(event.target.value);
    } else {
      setInputValue(event.target.value);
      setHistoryIndex(history.length);
    }
  };

  const handleInputKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape' && passwordPrompt) {
      event.preventDefault();
      setPasswordPrompt(null);
      setPasswordValue('');
      if (session) {
        setStatus(formatStatus(session.nickname, session.serverUrl, STATUS_HELP));
        setIsStatusError(false);
      }
      return;
    }

    if (passwordPrompt) {
      return;
    }

    if (event.key === 'ArrowUp') {
      event.preventDefault();
      setHistoryIndex((current) => {
        if (history.length === 0) {
          return current;
        }
        const nextIndex = Math.max(0, current - 1);
        const entry = history[nextIndex];
        if (entry !== undefined) {
          setInputValue(entry);
          return nextIndex;
        }
        return current;
      });
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      setHistoryIndex((current) => {
        if (history.length === 0) {
          return current;
        }
        const nextIndex = Math.min(history.length, current + 1);
        const entry = history[nextIndex];
        setInputValue(entry ?? '');
        return nextIndex;
      });
    }
  };

  const chatHeader = useMemo(() => {
    if (!session) {
      return null;
    }
    return (
      <div className="chat-layout__header">
        <div className="chat-layout__title">Cap'n Web Chat</div>
        <div className="chat-layout__session">
          <span className="chat-layout__session-label">Server:</span>
          <span className="chat-layout__session-value">{session.serverUrl}</span>
          <span className="chat-layout__session-label">Nick:</span>
          <span className="chat-layout__session-value">{session.nickname}</span>
        </div>
      </div>
    );
  }, [session]);

  return (
    <div className="app">
      {phase !== 'chat' && (
        <div className="app__center">
          <LoginForm
            defaultUrl={DEFAULT_BACKEND}
            initialUrl={rememberedUrl}
            connecting={connecting}
            redeeming={isRedeemingToken}
            error={connectError}
            savedToken={
              savedTokenInfo
                ? {
                    nickname: savedTokenInfo.nickname,
                    username: savedTokenInfo.username,
                    issuedAt: savedTokenInfo.issuedAt,
                    lastUsed: savedTokenInfo.lastUsed,
                  }
                : undefined
            }
            onConnect={handleConnect}
            onRedeemToken={handleRedeemToken}
            onClearToken={handleClearToken}
          />
        </div>
      )}

      {phase === 'chat' && session && (
        <div className="chat-layout">
          <div className="chat-layout__header-row">
            {chatHeader}
            <button className="chat-layout__logout" type="button" onClick={handleLogout}>
              Log out
            </button>
          </div>

          <div
            className="chat-layout__messages"
            ref={messagesRef}
            onScroll={handleMessagesScroll}
            style={{ maxHeight: `calc(${visibleLines} * 1.6rem + 2.5rem)` }}
          >
            {messageCount === 0 ? (
              <div className="chat-layout__empty">No messages yet.</div>
            ) : (
              displayMessages.map((message, index) => {
                const key = `${message.timestamp}-${index}`;
                const lines = message.body.split('\n');
                return (
                  <div key={key} className="message">
                    <span className="message__time">{formatTimestamp(message.timestamp)}</span>
                    <span className="message__from">{message.from}:</span>
                    <span className="message__body">
                      {lines.map((line, lineIndex) => (
                        <span key={lineIndex} className="message__line">
                          {line}
                          {lineIndex < lines.length - 1 && <br />}
                        </span>
                      ))}
                    </span>
                  </div>
                );
              })
            )}
          </div>

          <form className="chat-layout__input-form" onSubmit={handleInputSubmit}>
            <label className="chat-layout__input-label" htmlFor="chat-input">
              {passwordPrompt
                ? `Password for nickname '${passwordPrompt.nickname}'`
                : 'Message or command'}
            </label>
            <input
              id="chat-input"
              ref={inputRef}
              className="chat-layout__input"
              type={passwordPrompt ? 'password' : 'text'}
              value={passwordPrompt ? passwordValue : inputValue}
              onChange={handleInputChange}
              onKeyDown={handleInputKeyDown}
              placeholder={passwordPrompt ? 'Enter password and press Enter' : 'Type a message or /command'}
              autoComplete="off"
            />
            <div className="chat-layout__hint">
              {passwordPrompt ? 'Press Esc to cancel password entry.' : STATUS_HELP}
            </div>
          </form>

          <div className={`chat-layout__status${isStatusError ? ' chat-layout__status--error' : ''}`}>
            {status}
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
function readLastUrl(): string | null {
  if (!supportsStorage()) {
    return null;
  }
  try {
    return window.localStorage.getItem(LAST_URL_STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeLastUrl(url: string) {
  if (!supportsStorage()) {
    return;
  }
  try {
    window.localStorage.setItem(LAST_URL_STORAGE_KEY, url);
  } catch {
    // ignore
  }
}

function clearLastUrl() {
  if (!supportsStorage()) {
    return;
  }
  try {
    window.localStorage.removeItem(LAST_URL_STORAGE_KEY);
  } catch {
    // ignore
  }
}
