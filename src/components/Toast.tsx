import { useEffect, useState } from "react";

type ToastItem = {
  id: number;
  text: string;
  kind: "info" | "success" | "error";
};

let toastSeq = 0;
const listeners = new Set<(items: ToastItem[]) => void>();
let items: ToastItem[] = [];

function emit() {
  for (const fn of listeners) fn(items);
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
  setTimeout(() => {
    items = items.filter((t) => t.id !== id);
    emit();
  }, 2600);
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
