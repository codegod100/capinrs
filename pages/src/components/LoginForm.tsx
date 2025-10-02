import { FormEvent, useEffect, useState } from 'react';

type SavedTokenSummary = {
  nickname?: string;
  username?: string;
  issuedAt?: number;
  lastUsed?: number;
};

type LoginFormProps = {
  defaultUrl: string;
  initialUrl?: string;
  connecting: boolean;
  redeeming: boolean;
  error?: string | null;
  savedToken?: SavedTokenSummary;
  onConnect: (options: {
    url: string;
    nickname?: string;
    nicknamePassword?: string;
  }) => Promise<void>;
  onRedeemToken?: (options: { url: string }) => Promise<void>;
  onClearToken?: () => void;
};

export function LoginForm({
  defaultUrl,
  initialUrl,
  connecting,
  redeeming,
  error,
  savedToken,
  onConnect,
  onRedeemToken,
  onClearToken,
}: LoginFormProps) {
  const [url, setUrl] = useState(initialUrl ?? defaultUrl);
  const [nickname, setNickname] = useState('');
  const [nicknamePassword, setNicknamePassword] = useState('');
  const [localError, setLocalError] = useState<string | null>(null);

  const busy = connecting || redeeming;

  useEffect(() => {
    setUrl(initialUrl ?? defaultUrl);
  }, [initialUrl, defaultUrl]);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setLocalError(null);

    const trimmedNickname = nickname.trim();
    const trimmedUrl = url.trim();
    const trimmedPassword = nicknamePassword.trim();

    if (!trimmedUrl) {
      setLocalError('Please enter the server URL.');
      return;
    }

    if (trimmedPassword && !trimmedNickname) {
      setLocalError('Provide a nickname when supplying a NickServ password.');
      return;
    }

    await onConnect({
      url: trimmedUrl,
      nickname: trimmedNickname || undefined,
      nicknamePassword: trimmedPassword || undefined,
    });
  };

  const handleRedeemClick = async () => {
    if (!onRedeemToken) {
      return;
    }
    setLocalError(null);
    const trimmedUrl = url.trim();
    if (!trimmedUrl) {
      setLocalError('Please enter the server URL.');
      return;
    }
    await onRedeemToken({ url: trimmedUrl });
  };

  const handleClearToken = () => {
    onClearToken?.();
  };

  const displayError = localError || error;

  return (
    <form className="login-form" onSubmit={handleSubmit}>
      <h1>Cap'n Web Chat</h1>
      <p className="login-form__description">
        Connect to your Cap'n Web backend from Cloudflare Pages. Leave the nickname blank to
        generate a random one. To auto-identify with NickServ, provide both a nickname and password.
      </p>

      <label className="login-form__label" htmlFor="url-input">
        Server URL
      </label>
      <input
        id="url-input"
        className="login-form__input"
        type="text"
        value={url}
        onChange={(event) => setUrl(event.target.value)}
        placeholder="wss://example.com"
        autoComplete="off"
        disabled={busy}
      />

      <label className="login-form__label" htmlFor="nickname-input">
        Nickname (optional)
      </label>
      <input
        id="nickname-input"
        className="login-form__input"
        type="text"
        value={nickname}
        onChange={(event) => setNickname(event.target.value)}
        placeholder="Leave blank for a random nickname"
        autoComplete="off"
        disabled={busy}
      />

      <label className="login-form__label" htmlFor="nickname-password-input">
        NickServ password (optional)
      </label>
      <input
        id="nickname-password-input"
        className="login-form__input"
        type="password"
        value={nicknamePassword}
        onChange={(event) => setNicknamePassword(event.target.value)}
        placeholder="Required only when a nickname is supplied"
        autoComplete="current-password"
        disabled={busy}
      />

      {savedToken && (
        <div className="login-form__token-card">
          <div className="login-form__token-text">
            Saved token for{' '}
            <span className="login-form__token-identity">
              {savedToken.nickname ?? savedToken.username ?? 'unknown user'}
            </span>
            {savedToken.lastUsed && (
              <span className="login-form__token-meta">
                {' '}
                • Last used {new Date(savedToken.lastUsed).toLocaleString()}
              </span>
            )}
          </div>
          <div className="login-form__token-actions">
            <button
              type="button"
              className="login-form__token-button"
              onClick={handleRedeemClick}
              disabled={busy}
            >
              {redeeming ? 'Redeeming…' : 'Sign in with token'}
            </button>
            <button
              type="button"
              className="login-form__token-remove"
              onClick={handleClearToken}
              disabled={busy}
            >
              Remove token
            </button>
          </div>
        </div>
      )}

      {displayError && <div className="login-form__error" role="alert">{displayError}</div>}

      <button className="login-form__button" type="submit" disabled={busy}>
        {connecting ? 'Connecting…' : redeeming ? 'Redeeming…' : 'Connect'}
      </button>
    </form>
  );
}
