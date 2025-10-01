import * as readline from 'readline';
import React, { useState, useEffect, useCallback } from 'react';
import { render, Text, Box, useInput, useApp } from 'ink';
import { newWebSocketRpcSession, RpcTarget } from 'capnweb';

const DEFAULT_BACKEND = 'ws://localhost:8787';

// Local RPC target that the server can call
class ChatClient extends RpcTarget {
  public onMessage?: (message: { from: string; body: string; timestamp: number }) => void;

  receiveMessage(message: { from: string; body: string; timestamp: number }) {
    console.log('Client receiveMessage called with:', message);
    // This will be visible in the UI
    if (this.onMessage) {
      this.onMessage(message);
    } else {
      console.log(`${message.from}: ${message.body}`);
    }
  }
}

interface CliOptions {
  url: string;
}

interface Session {
  username: string;
  capabilityId: number;
}

enum LoopAction {
  Continue,
  Exit,
}



function usage(): void {
  console.log(`
Usage: npm run dev:client -- [OPTIONS]

Options:
  --url <URL>    Override the Cap'n Web endpoint
  -h, --help     Show this message

Environment:
  CAPINRS_SERVER_HOST   Override the default backend (${DEFAULT_BACKEND})

After launch you'll be prompted for username/password, the server will
hand back a dedicated chat capability, and you can chat interactively.
Commands: /help, /auth, /receive, /whoami, /quit.
`);
}

function ensureScheme(raw: string, fallback: string): string {
  if (raw.includes('://')) {
    return raw;
  } else {
    return `${fallback}${raw}`;
  }
}

function normalizeEndpoint(raw: string, defaultScheme: string): string {
  return ensureScheme(raw, defaultScheme);
}

function parseCli(): CliOptions {
  const args = process.argv.slice(2);
  let urlOverride: string | undefined;

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    switch (arg) {
      case '--help':
      case '-h':
        usage();
        process.exit(0);
        break;
      case '--url':
      case '--host':
        if (i + 1 >= args.length) {
          console.error('`--url` requires a value');
          process.exit(1);
        }
        urlOverride = args[i + 1];
        i++;
        break;
      default:
        if (arg.startsWith('-')) {
          console.error(`Unrecognized flag \`${arg}\``);
          process.exit(1);
        } else {
          console.error(`Unexpected argument \`${arg}\``);
          process.exit(1);
        }
    }
  }

  const envOverride = process.env.CAPINRS_SERVER_HOST;
  const rawTarget = urlOverride || envOverride || DEFAULT_BACKEND;
  const url = normalizeEndpoint(rawTarget, 'http://');

  return { url };
}

function prompt(question: string): Promise<string> {
  return new Promise((resolve) => {
    const rl = readline.createInterface({
      input: process.stdin,
      output: process.stdout,
    });
    rl.question(question, (answer) => {
      rl.close();
      resolve(answer.trim());
    });
  });
}

// Using capnweb library for RPC over WebSocket
type RpcSessionHandles = {
  remoteMain: any;
  client: ChatClient;
};

// Using capnweb library for RPC over WebSocket
async function createRpcSession(url: string): Promise<RpcSessionHandles> {
  // Create a local RPC target that the server can call
  const localClient = new ChatClient();

  // Create a WebSocket RPC session using capnweb, passing our local client
  const remoteMain = await newWebSocketRpcSession(url, localClient);
  return { remoteMain, client: localClient };
}

// Moved to ChatApp component

interface ChatAppProps {
  client: ChatClient;
  remoteMain: any;
  session: Session;
}

interface Message {
  from: string;
  body: string;
  timestamp: number;
}

function ChatApp({ client, remoteMain, session }: ChatAppProps) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [status, setStatus] = useState('Connected');
  const [isError, setIsError] = useState(false);
  const { exit } = useApp();

  // Set up message handler and load initial messages
  useEffect(() => {
    client.onMessage = (message) => {
      setMessages(prev => [...prev, message]);
    };

    // Load initial messages
    const loadInitialMessages = async () => {
      try {
        const messagesResult = await remoteMain.receiveMessages(session.capabilityId);
        messagesResult.messages.forEach((msg: any) => {
          setMessages(prev => [...prev, {
            from: msg.from,
            body: msg.body,
            timestamp: msg.timestamp || Date.now()
          }]);
        });
        setStatus(`Connected as ${session.username}`);
        setIsError(false);
        
        // Add welcome message
        setMessages(prev => [...prev, {
          from: 'System',
          body: `Welcome, ${session.username}! Type /help for available commands.`,
          timestamp: Date.now()
        }]);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setStatus(`Failed to get initial messages: ${message}`);
        setIsError(true);
      }
    };

    loadInitialMessages();
  }, [client, remoteMain, session.capabilityId]);

  // Handle user input
  const handleSubmit = useCallback(async (text: string) => {
    const trimmed = text.trim();
    if (trimmed.length === 0) {
      return;
    }

    if (!trimmed.startsWith('/')) {
      // Send message using capnweb RPC
      try {
        await remoteMain.sendMessage(session.capabilityId, trimmed);
        setStatus(`Connected as ${session.username}`);
        setIsError(false);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setStatus(`Failed to send message: ${message}`);
        setIsError(true);
      }
      return;
    }

    const parts = trimmed.split(/\s+/);
    const command = parts[0];

    switch (command) {
      case '/quit':
      case '/exit':
        exit();
        return;
      case '/help':
        setMessages(prev => [...prev, {
          from: 'System',
          body: `Commands:
  /help                  Show this help
  /whoami                Show current session
  /receive               Fetch and display messages
  /quit                  Exit the client
Messages without a leading slash are broadcast to the chat.`,
          timestamp: Date.now()
        }]);
        return;
      case '/whoami':
        try {
          const result = await remoteMain.whoami(session.capabilityId);
          setMessages(prev => [...prev, {
            from: 'System',
            body: `You are ${result.username}`,
            timestamp: Date.now()
          }]);
          setStatus(`Authenticated as ${result.username}`);
          setIsError(false);
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          setStatus(`Whoami failed: ${message}`);
          setIsError(true);
        }
        return;
      case '/receive':
        try {
          const messagesResult = await remoteMain.receiveMessages(session.capabilityId);
          messagesResult.messages.forEach((msg: any) => {
            setMessages(prev => [...prev, {
              from: msg.from,
              body: msg.body,
              timestamp: msg.timestamp || Date.now()
            }]);
          });
          setStatus('Fetched recent messages');
          setIsError(false);
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          setStatus(`Failed to receive messages: ${message}`);
          setIsError(true);
        }
        return;
      default:
        setMessages(prev => [...prev, {
          from: 'System',
          body: `Unknown command \`${command}\`. Type /help for a list of commands.`,
          timestamp: Date.now()
        }]);
        return;
    }
  }, [remoteMain, session, exit]);

  // Handle keyboard input
  useInput((inputChar: string, key: any) => {
    if (key.ctrl && inputChar === 'c') {
      exit();
      return;
    }

    if (key.return) {
      handleSubmit(input);
      setInput('');
      return;
    }

    if (key.backspace || key.delete) {
      setInput(prev => prev.slice(0, -1));
      return;
    }

    if (inputChar && !key.ctrl && !key.meta) {
      setInput(prev => prev + inputChar);
    }
  });

  return (
    <Box flexDirection="column" height="100%" width="100%">
      {/* Messages */}
      <Box flexDirection="column" flexGrow={1} borderStyle="round" borderColor="cyan">
        <Text color="cyan"> Messages </Text>
        {messages.map((msg, index) => (
          <Text key={index}>
            <Text color="green">{msg.from}:</Text> {msg.body}
          </Text>
        ))}
      </Box>

      {/* Input */}
      <Box borderStyle="round" borderColor="yellow">
        <Text color="yellow"> Input </Text>
        <Text>{input}</Text>
      </Box>

      {/* Status */}
      <Box backgroundColor={isError ? "red" : "blue"}>
        <Text color="white"> {status} </Text>
      </Box>
    </Box>
  );
}

// No longer needed - Ink handles UI

async function main() {
  const options = parseCli();

  console.log(`Connecting to ${options.url}`);

  let remoteMain: any;
  let client: ChatClient;
  try {
    const sessionHandles = await createRpcSession(options.url);
    remoteMain = sessionHandles.remoteMain;
    client = sessionHandles.client;
  } catch (error) {
    console.error('Connection failed:', error instanceof Error ? error.message : String(error));
    process.exit(1);
  }

  const username = await prompt('Username: ');
  const password = await prompt('Password: ');

  let sessionId: number;
  try {
    // Authenticate using capnweb RPC
    const authResult: any = await remoteMain.auth(username, password);
    sessionId = authResult.session.id;
    console.log(`Welcome, ${authResult.user}! Type /help for available commands.`);
  } catch (error) {
    console.error('Authentication failed:', error instanceof Error ? error.message : String(error));
    process.exit(1);
  }

  const session: Session = {
    username,
    capabilityId: sessionId,
  };

  // Render the Ink app
  render(<ChatApp client={client} remoteMain={remoteMain} session={session} />);
}

// Run the client
main().catch(console.error);