import { useEffect, useState } from "react";
import { api, type Account } from "./lib/api";
import { useVideoState } from "./lib/videoState";
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
  // 记录用户曾访问过哪些 tab，配合 hidden 属性实现"懒挂载且永不卸载"：
  // - 用户首次切到某 tab 时挂载它，并把它放进 mounted 集合
  // - 再次离开该 tab 时不卸载，仅用 hidden 隐藏，保留其内部 local state
  // 这样讨论评论页 / 自动作业页等的"已选课程、勾选状态、执行结果"等
  // 不会因切 tab 而丢失（之前是条件渲染 → 组件卸载 → state 清零）。
  const [mounted, setMounted] = useState<Set<NavKey>>(() => new Set([active]));
  const videoState = useVideoState();

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

  // 当 active 切换到一个从未挂载过的 tab 时，把它加入 mounted。
  // 已挂载的不动，保留其 state。
  useEffect(() => {
    setMounted((s) => {
      if (s.has(active)) return s;
      const next = new Set(s);
      next.add(active);
      return next;
    });
  }, [active]);

  if (!bootstrapped) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-parchment">
        <div className="text-body text-ink-muted-80">加载中…</div>
      </div>
    );
  }

  if (!current || forceLogin) {
    // forceLogin=true 表示用户从账号管理页主动进入扫码加号场景，
    // 此时应支持点击"取消"返回账号管理页（current 必然存在）。
    // 首次登录（current=null）则不传 onCancel，保持原行为。
    return (
      <>
        <ToastHost />
        <LoginPage
          onLoggedIn={refreshCurrent}
          onCancel={
            forceLogin && current ? () => setForceLogin(false) : undefined
          }
        />
      </>
    );
  }

  return (
    <div className="min-h-screen flex bg-parchment">
      <ToastHost />
      <Sidebar active={active} onChange={setActive} currentName={current.nickname} />
      <main className="flex-1 overflow-auto bg-white">
        {/*
          以下每个 tab 都遵循"首次访问后保持挂载、用 hidden 控制可见性"的规则。
          这样从其他 tab 切回时，已加载的课程、勾选状态、执行结果、运行中任务
          等本地 state 都不会被清空。视频页继续复用 useVideoState 持有的全局状态。
        */}
        {mounted.has("home") && (
          <div hidden={active !== "home"}>
            <HomePage current={current} />
          </div>
        )}
        {mounted.has("courses") && (
          <div hidden={active !== "courses"}>
            <CoursesPage current={current} />
          </div>
        )}
        {mounted.has("video") && (
          <div hidden={active !== "video"}>
            <VideoPage state={videoState} />
          </div>
        )}
        {mounted.has("forum") && (
          <div hidden={active !== "forum"}>
            <ForumPage />
          </div>
        )}
        {mounted.has("homework") && (
          <div hidden={active !== "homework"}>
            <HomeworkPage />
          </div>
        )}
        {mounted.has("accounts") && (
          <div hidden={active !== "accounts"}>
            <AccountsPage
              onChanged={refreshCurrent}
              onLoginAgain={() => setForceLogin(true)}
            />
          </div>
        )}
        {mounted.has("settings") && (
          <div hidden={active !== "settings"}>
            <SettingsPage />
          </div>
        )}
      </main>
    </div>
  );
}
