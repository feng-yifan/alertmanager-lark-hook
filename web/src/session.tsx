/** Session bootstrap + login/logout, shared through React context. */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import * as api from './api';
import { setUnauthorizedHandler } from './api';

export interface SessionUser {
  username: string;
  /**
   * `GET /auth/session` 也返回真实的 `must_change_password`，因此刷新页面后
   * 「需修改密码」标记与强制跳转仍然有效；登录响应同样携带该标记。
   */
  must_change_password: boolean;
}

export type SessionStatus = 'loading' | 'anonymous' | 'authenticated';

export interface SessionContextValue {
  status: SessionStatus;
  user: SessionUser | null;
  login: (username: string, password: string) => Promise<SessionUser>;
  logout: () => Promise<void>;
  refresh: () => Promise<void>;
  markPasswordChanged: () => void;
}

const SessionContext = createContext<SessionContextValue | null>(null);

export function SessionProvider({ children }: { children: ReactNode }): JSX.Element {
  const [status, setStatus] = useState<SessionStatus>('loading');
  const [user, setUser] = useState<SessionUser | null>(null);

  const bootstrap = useCallback(async () => {
    try {
      const session = await api.getSession();
      if (session.authenticated) {
        setUser({
          username: session.username,
          must_change_password: session.must_change_password ?? false,
        });
        setStatus('authenticated');
      } else {
        setUser(null);
        setStatus('anonymous');
      }
    } catch {
      setUser(null);
      setStatus('anonymous');
    }
  }, []);

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  // A 401 from any authenticated endpoint drops the session; the router guard
  // (`RequireAuth`) then redirects to /login.
  useEffect(() => {
    setUnauthorizedHandler(() => {
      setUser(null);
      setStatus((current) => (current === 'loading' ? 'anonymous' : 'anonymous'));
    });
    return () => setUnauthorizedHandler(null);
  }, []);

  const login = useCallback(async (username: string, password: string) => {
    const response = await api.login({ username, password });
    const nextUser: SessionUser = {
      username: response.username,
      must_change_password: response.must_change_password,
    };
    setUser(nextUser);
    setStatus('authenticated');
    return nextUser;
  }, []);

  const logout = useCallback(async () => {
    try {
      await api.logout();
    } finally {
      setUser(null);
      setStatus('anonymous');
    }
  }, []);

  const markPasswordChanged = useCallback(() => {
    setUser((previous) => (previous ? { ...previous, must_change_password: false } : previous));
  }, []);

  const value = useMemo<SessionContextValue>(
    () => ({ status, user, login, logout, refresh: bootstrap, markPasswordChanged }),
    [status, user, login, logout, bootstrap, markPasswordChanged],
  );

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

export function useSession(): SessionContextValue {
  const value = useContext(SessionContext);
  if (!value) throw new Error('useSession must be used inside <SessionProvider>');
  return value;
}
