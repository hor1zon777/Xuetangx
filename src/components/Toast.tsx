import { useEffect, useState } from "react";

type ToastItem = {
  id: number;
  text: string;
  kind: "info" | "success" | "error";
};

const TOAST_TTL_MS = 2600;

let toastSeq = 0;
const listeners = new Set<(items: ToastItem[]) => void>();
let items: ToastItem[] = [];
const timers = new Map<number, ReturnType<typeof setTimeout>>();

function emit() {
  // 拷贝一份再遍历，避免 listener 内同步退订时迭代器失效
  const snapshot = [...listeners];
  for (const fn of snapshot) fn(items);
}

function dismiss(id: number) {
  const timer = timers.get(id);
  if (timer !== undefined) {
    clearTimeout(timer);
    timers.delete(id);
  }
  const before = items.length;
  items = items.filter((t) => t.id !== id);
  if (items.length !== before) emit();
}

export const toast = {
  info: (text: string) => push(text, "info"),
  success: (text: string) => push(text, "success"),
  error: (text: string) => push(text, "error"),
};

function push(text: string, kind: ToastItem["kind"]) {
  const id = ++toastSeq;
  items = [...items, { id, text, kind }];
  emit();
  const timer = setTimeout(() => dismiss(id), TOAST_TTL_MS);
  timers.set(id, timer);
}

export function ToastHost() {
  const [list, setList] = useState<ToastItem[]>(items);
  useEffect(() => {
    listeners.add(setList);
    return () => {
      listeners.delete(setList);
    };
  }, []);
  if (!list.length) return null;
  return (
    <div className="toast flex flex-col items-center gap-2 pointer-events-none">
      {list.map((t) => (
        <div
          key={t.id}
          className={
            "min-w-[200px] max-w-[420px] px-5 py-2.5 rounded-pill text-caption text-center shadow-product backdrop-blur-md pointer-events-auto " +
            (t.kind === "success"
              ? "bg-action-blue text-white"
              : t.kind === "error"
              ? "bg-[#cc2b2b] text-white"
              : "bg-ink/90 text-white")
          }
        >
          {t.text}
        </div>
      ))}
    </div>
  );
}
