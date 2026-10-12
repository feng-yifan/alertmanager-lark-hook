import { useState, type FormEvent } from 'react';
import { Navigate, useLocation, useNavigate } from 'react-router-dom';
import { errorMessage } from '../api';
import { Banner, Button, Field, Input } from '../components/ui';
import { useSession } from '../session';
import { useToast } from '../toast';

interface LocationState {
  from?: string;
}

export default function LoginPage(): JSX.Element {
  const { status, user, login } = useSession();
  const navigate = useNavigate();
  const location = useLocation();
  const toast = useToast();

  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const requested = (location.state as LocationState | null)?.from;
  const target = requested && requested !== '/login' ? requested : '/';

  if (status === 'authenticated') {
    return <Navigate to={user?.must_change_password ? '/settings' : target} replace />;
  }

  const onSubmit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const nextUser = await login(username.trim(), password);
      toast.success(`欢迎回来，${nextUser.username}`);
      navigate(nextUser.must_change_password ? '/settings' : target, { replace: true });
    } catch (cause) {
      setError(cause);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="login-page">
      <div className="login-card">
        <h1>登录管理后台</h1>
        <div className="login-sub">alertmanager-lark-hook · 飞书告警投递</div>

        {error ? <Banner tone="error">{errorMessage(error)}</Banner> : null}

        <form onSubmit={(event) => void onSubmit(event)}>
          <Field label="用户名" htmlFor="login-username" required>
            <Input
              id="login-username"
              value={username}
              autoComplete="username"
              autoFocus
              required
              onChange={(event) => setUsername(event.target.value)}
            />
          </Field>
          <Field label="密码" htmlFor="login-password" required>
            <Input
              id="login-password"
              type="password"
              value={password}
              autoComplete="current-password"
              required
              onChange={(event) => setPassword(event.target.value)}
            />
          </Field>
          <Button type="submit" variant="primary" loading={busy} className="mt-8" style={{ width: '100%' }}>
            登录
          </Button>
        </form>

        <p className="faint small mt-16 mb-0">
          会话使用 HttpOnly Cookie（<code>alkh_session</code>），接口基址 <code>/api/v1</code>。
        </p>
      </div>
    </div>
  );
}
