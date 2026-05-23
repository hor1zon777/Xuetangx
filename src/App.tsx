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
import { BankPage } from "./pages/Bank";
import { AccountsPage } from "./pages/Accounts";
import { SettingsPage } from "./pages/Settings";
import { AboutPage } from "./pages/About";

/**
 * 已登录场景的外壳：Sidebar + 所有功能 tab + 全局视频状态。
 *
 * 关键设计：本组件 **不持有跨账号通用 state**（只有 active/mounted 这种 UI 偏好），
 * 所以可以让 App 在切换账号时通过 `key={current.user_id}` 把整个 AppShell 卸载并重建。
 * 这样：
 *   - 每个 tab 组件自己的 useState（已选课程、章节、勾选、执行结果等）全部归零
 *   - 各 tab 的 useEffect([]) 重新跑一次，重新从后端拉当前账号的数据
 *   - useVideoState 这种 App 层全局 hook 也跟着重建（旧账号的运行中任务前端
 *     列表清空，但后端运行中的任务不被强制中止）
 *
 * 不会破坏"同账号下切 tab 保留 state"的体验：account 不变时 key 不变、组件不卸载。
 */
function AppShell({
  current,
  onLoginAgain,
  refreshCurrent,
  active,
  setActive,
}: {
  current: Account;
  onLoginAgain: () => void;
  refreshCurrent: () => void;
  active: NavKey;
  setActive: (k: NavKey) => void;
}) {
  // 记录用户曾访问过哪些 tab，配合 hidden 属性实现"懒挂载且永不卸载"：
  // - 用户首次切到某 tab 时挂载它，并把它放进 mounted 集合
  // - 再次离开该 tab 时不卸载，仅用 hidden 隐藏，保留其内部 local state
  // 这样讨论评论页 / 自动作业页等的"已选课程、勾选状态、执行结果"等
  // 不会因切 tab 而丢失（之前是条件渲染 → 组件卸载 → state 清零）。
  //
  // 注：mounted 是 shell 内部 state，被 key={current.user_id} 控制重建。
  // 切换账号后这里会从空集重新初始化，只有当前 active 的 tab 会被立刻挂载，
  // 避免一次性把所有 tab 并发拉数据。其它 tab 等用户切过去时才懒挂载。
  const [mounted, setMounted] = useState<Set<NavKey>>(() => new Set([active]));
  const videoState = useVideoState();

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

  return (
    <div className="min-h-screen flex bg-parchment">
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
        {mounted.has("bank") && (
          <div hidden={active !== "bank"}>
            <BankPage />
          </div>
        )}
        {mounted.has("accounts") && (
          <div hidden={active !== "accounts"}>
            <AccountsPage
              onChanged={refreshCurrent}
              onLoginAgain={onLoginAgain}
            />
          </div>
        )}
        {mounted.has("settings") && (
          <div hidden={active !== "settings"}>
            <SettingsPage />
          </div>
        )}
        {mounted.has("about") && (
          <div hidden={active !== "about"}>
            <AboutPage />
          </div>
        )}
      </main>
    </div>
  );
}

export default function App() {
  const [bootstrapped, setBootstrapped] = useState(false);
  const [current, setCurrent] = useState<Account | null>(null);
  const [forceLogin, setForceLogin] = useState(false);
  // active 提到 App 层，不受 AppShell 的 key 重建影响：
  // 切换账号后保留用户当前所在 tab，不强行跳回首页。
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

  // 关键点：AppShell 上挂 `key={current.user_id}`。
  // 切换账号时（refreshCurrent → setCurrent → 新 user_id）整个 shell 卸载重建，
  // 所有 tab 的本地 state（已选课程、章节、勾选、执行结果、视频任务列表 …）全部归零，
  // 各自的 useEffect([]) 会重新拉当前账号的数据，从根本上避免显示上一个账号的内容。
  return (
    <>
      <ToastHost />
      <AppShell
        key={current.user_id}
        current={current}
        active={active}
        setActive={setActive}
        refreshCurrent={refreshCurrent}
        onLoginAgain={() => setForceLogin(true)}
      />
    </>
  );
}
