import { useEffect, useState } from "react";
import { api, type Account } from "./lib/api";
import { Sidebar, type NavKey } from "./components/Sidebar";
import { ToastHost } from "./components/Toast";
import { LoginPage } from "./pages/Login";
import { HomePage } from "./pages/Home";
import { CoursesPage } from "./pages/Courses";
import { VideoPage } from "./pages/Video";
import { ForumPage } from "./pages/Forum";
import { HomeworkPage } from "./pages/Homework";
import { AccountsPage } from "./pages/Accounts";
import { SettingsPage } from "./pages/Settings";

export default function App() {
  const [bootstrapped, setBootstrapped] = useState(false);
  const [current, setCurrent] = useState<Account | null>(null);
  const [forceLogin, setForceLogin] = useState(false);
  const [active, setActive] = useState<NavKey>("home");

  const refreshCurrent = async () => {
    const cur = await api.currentAccount();
    setCurrent(cur);
    setForceLogin(false);
  };

  useEffect(() => {
    (async () => {
      const cur = await api.currentAccount();
      setCurrent(cur);
      setBootstrapped(true);
    })();
  }, []);

  if (!bootstrapped) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-parchment">
        <div className="text-body text-ink-muted-80">加载中…</div>
      </div>
    );
  }

  if (!current || forceLogin) {
    return (
      <>
        <ToastHost />
        <LoginPage onLoggedIn={refreshCurrent} />
      </>
    );
  }

  return (
    <div className="min-h-screen flex bg-parchment">
      <ToastHost />
      <Sidebar active={active} onChange={setActive} currentName={current.nickname} />
      <main className="flex-1 overflow-auto bg-white">
        {active === "home" && <HomePage current={current} />}
        {active === "courses" && <CoursesPage current={current} />}
        {active === "video" && <VideoPage />}
        {active === "forum" && <ForumPage />}
        {active === "homework" && <HomeworkPage />}
        {active === "accounts" && (
          <AccountsPage
            onChanged={refreshCurrent}
            onLoginAgain={() => setForceLogin(true)}
          />
        )}
        {active === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}
