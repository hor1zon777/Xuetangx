import clsx from "clsx";

type NavKey =
  | "home"
  | "courses"
  | "video"
  | "forum"
  | "homework"
  | "bank"
  | "accounts"
  | "settings"
  | "about";

const items: { key: NavKey; label: string }[] = [
  { key: "home", label: "概览" },
  { key: "courses", label: "我的课程" },
  { key: "video", label: "自动观看" },
  { key: "forum", label: "讨论 / 图文" },
  { key: "homework", label: "自动作业" },
  { key: "bank", label: "题库" },
  { key: "accounts", label: "账号管理" },
  { key: "settings", label: "设置" },
  { key: "about", label: "关于" },
];

export function Sidebar({
  active,
  onChange,
  currentName,
}: {
  active: NavKey;
  onChange: (k: NavKey) => void;
  currentName?: string;
}) {
  return (
    <aside className="w-[220px] shrink-0 bg-ink text-white flex flex-col">
      <div className="px-6 py-6">
        <div className="text-tagline font-display">学堂在线</div>
        <div className="text-fine text-white/60 mt-1">自动学习助手</div>
      </div>
      <nav className="flex-1 px-2">
        {items.map((it) => (
          <button
            key={it.key}
            onClick={() => onChange(it.key)}
            className={clsx(
              "block w-full text-left text-body px-4 py-3 rounded-sm mb-1 transition",
              active === it.key
                ? "bg-white/10 text-white"
                : "text-white/70 hover:text-white hover:bg-white/5"
            )}
          >
            {it.label}
          </button>
        ))}
      </nav>
      <div className="px-6 py-4 text-fine text-white/60 border-t border-white/10">
        当前账号
        <div className="text-caption text-white mt-1 truncate">
          {currentName || "未登录"}
        </div>
      </div>
    </aside>
  );
}

export type { NavKey };
