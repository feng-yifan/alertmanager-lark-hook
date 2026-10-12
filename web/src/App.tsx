import { useState } from 'react';
import { NavLink, Navigate, Outlet, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import { Button, Loading, PageHeader } from './components/ui';
import AlertGroupDetailPage from './pages/AlertGroupDetailPage';
import AlertGroupsPage from './pages/AlertGroupsPage';
import CardTemplateEditorPage from './pages/CardTemplateEditorPage';
import CardTemplatesPage from './pages/CardTemplatesPage';
import ChannelDetailPage from './pages/ChannelDetailPage';
import ChannelsPage from './pages/ChannelsPage';
import DeliveriesPage from './pages/DeliveriesPage';
import LoginPage from './pages/LoginPage';
import OverviewPage from './pages/OverviewPage';
import SettingsPage from './pages/SettingsPage';
import { useSession } from './session';

const NAV: Array<{ to: string; label: string; end?: boolean }> = [
  { to: '/', label: '概览', end: true },
  { to: '/channels', label: '渠道' },
  { to: '/card-templates', label: '卡片模板' },
  { to: '/alert-groups', label: '告警分组' },
  { to: '/deliveries', label: '投递记录' },
  { to: '/settings', label: '设置' },
];

function titleFor(pathname: string): string {
  if (pathname === '/') return '概览';
  if (pathname.startsWith('/channels')) return '渠道';
  if (pathname.startsWith('/card-templates')) return '卡片模板';
  if (pathname.startsWith('/alert-groups')) return '告警分组';
  if (pathname.startsWith('/deliveries')) return '投递记录';
  if (pathname.startsWith('/settings')) return '设置';
  return '管理后台';
}

function RequireAuth(): JSX.Element {
  const { status } = useSession();
  const location = useLocation();

  if (status === 'loading') {
    return (
      <div className="login-page">
        <Loading label="正在校验会话…" />
      </div>
    );
  }
  if (status === 'anonymous') {
    return <Navigate to="/login" replace state={{ from: `${location.pathname}${location.search}` }} />;
  }
  return <Outlet />;
}

function AppLayout(): JSX.Element {
  const { user, logout } = useSession();
  const navigate = useNavigate();
  const location = useLocation();
  const [busy, setBusy] = useState(false);

  const handleLogout = async () => {
    setBusy(true);
    try {
      await logout();
      navigate('/login', { replace: true });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="sidebar-brand">
          <strong>AlertManager → 飞书</strong>
          <span>alertmanager-lark-hook</span>
        </div>
        <nav className="nav">
          {NAV.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.end ?? false}
              className={({ isActive }) => (isActive ? 'active' : undefined)}
            >
              {item.label}
            </NavLink>
          ))}
        </nav>
        <div className="sidebar-footer">
          <span>{user?.username ?? '—'}</span>
          <Button size="sm" onClick={() => void handleLogout()} loading={busy}>
            退出登录
          </Button>
        </div>
      </aside>
      <div className="main">
        <header className="topbar">
          <div className="topbar-title">{titleFor(location.pathname)}</div>
          <div className="topbar-user">
            <span>{user?.username}</span>
            {user?.must_change_password ? <span className="badge badge-warn">需修改密码</span> : null}
          </div>
        </header>
        <main className="content">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

function NotFoundPage(): JSX.Element {
  const navigate = useNavigate();
  return (
    <>
      <PageHeader title="页面不存在" subtitle="请求的路径没有对应的管理页面。" />
      <Button variant="primary" onClick={() => navigate('/')}>
        返回概览
      </Button>
    </>
  );
}

export default function App(): JSX.Element {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route element={<RequireAuth />}>
        <Route element={<AppLayout />}>
          <Route path="/" element={<OverviewPage />} />
          <Route path="/channels" element={<ChannelsPage />} />
          <Route path="/channels/:id" element={<ChannelDetailPage />} />
          <Route path="/card-templates" element={<CardTemplatesPage />} />
          <Route path="/card-templates/new" element={<CardTemplateEditorPage />} />
          <Route path="/card-templates/:id" element={<CardTemplateEditorPage />} />
          <Route path="/alert-groups" element={<AlertGroupsPage />} />
          <Route path="/alert-groups/:id" element={<AlertGroupDetailPage />} />
          <Route path="/deliveries" element={<DeliveriesPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="*" element={<NotFoundPage />} />
        </Route>
      </Route>
    </Routes>
  );
}
