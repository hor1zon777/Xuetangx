import { useEffect, useMemo, useState } from "react";
import { api, type Course, type LeafNode } from "../lib/api";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";
import { toast } from "../components/Toast";

type LeafExtra = {
  exercise_id?: number;
  resolved?: boolean; // 已尝试解析（无论成功或失败）
};

type ResultGroup = {
  leaf_id: number;
  leaf_name: string;
  status: "running" | "done" | "error";
  error?: string;
  items: any[];
};

export function HomeworkPage() {
  const [courses, setCourses] = useState<Course[]>([]);
  const [selected, setSelected] = useState<Course | null>(null);
  const [leaves, setLeaves] = useState<LeafNode[]>([]);
  const [extra, setExtra] = useState<Record<number, LeafExtra>>({});
  const [loading, setLoading] = useState(false);
  const [resolving, setResolving] = useState(false);
  const [running, setRunning] = useState(false);
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [results, setResults] = useState<ResultGroup[]>([]);

  const allHomeworkLeaves = useMemo(
    () => leaves.filter((l) => l.leaf_type === 6 || l.leaf_type === 7 || l.leaf_type === 3),
    [leaves]
  );

  // 仅展示真正有 exercise_id 的节点；尚未解析完成的节点暂保留以显示"解析中"
  const visibleLeaves = useMemo(
    () =>
      allHomeworkLeaves.filter((l) => {
        const ex = extra[l.id];
        if (!ex) return true; // 还没解析过，显示占位
        if (!ex.resolved) return true;
        return ex.exercise_id != null && ex.exercise_id > 0;
      }),
    [allHomeworkLeaves, extra]
  );

  useEffect(() => {
    api.listCourses().then(setCourses);
  }, []);

  const loadLeaves = async (c: Course) => {
    setSelected(c);
    setLoading(true);
    setExtra({});
    setPicked(new Set());
    try {
      const ls = await api.listChapters(c.classroom_id, c.sign);
      setLeaves(ls);

      const hwLeafs = ls.filter(
        (x) => x.leaf_type === 6 || x.leaf_type === 7 || x.leaf_type === 3
      );
      if (hwLeafs.length === 0) return;

      setResolving(true);
      try {
        const map = await api.batchExerciseIds(
          c.classroom_id,
          c.sign,
          hwLeafs.map((l) => l.id)
        );
        const next: Record<number, LeafExtra> = {};
        for (const l of hwLeafs) {
          const ex = (map as any)[String(l.id)] ?? (map as any)[l.id];
          next[l.id] = {
            exercise_id: typeof ex === "number" ? ex : undefined,
            resolved: true,
          };
        }
        setExtra(next);
      } catch (e: any) {
        toast.error(`习题节点解析失败：${e}`);
      } finally {
        setResolving(false);
      }
    } finally {
      setLoading(false);
    }
  };

  const toggle = (id: number) => {
    const n = new Set(picked);
    if (n.has(id)) n.delete(id);
    else n.add(id);
    setPicked(n);
  };

  const selectAll = () => {
    setPicked(
      new Set(
        visibleLeaves
          .filter((l) => extra[l.id]?.exercise_id)
          .map((l) => l.id)
      )
    );
  };

  const clearSelection = () => setPicked(new Set());

  const startAll = async () => {
    if (!selected || picked.size === 0) return;
    setRunning(true);
    const targets = visibleLeaves.filter(
      (l) => picked.has(l.id) && extra[l.id]?.exercise_id
    );
    // 初始化 result groups（按节点分组，便于实时看进度）
    const initialGroups: ResultGroup[] = targets.map((l) => ({
      leaf_id: l.id,
      leaf_name: l.name,
      status: "running",
      items: [],
    }));
    setResults(initialGroups);
    toast.info(`已提交 ${targets.length} 个节点的作业`);

    let totalOk = 0;
    let totalQ = 0;
    // 顺序执行以避免对大模型与服务端造成并发压力
    for (const l of targets) {
      try {
        const out = await api.autoHomeworkLeaf({
          leaf_id: l.id,
          classroom_id: selected.classroom_id,
          sku_id: selected.sku_id,
          exercise_id: extra[l.id]!.exercise_id!,
          sign: selected.sign,
        });
        totalQ += out.length;
        totalOk += out.filter((r: any) => r?.submit?.is_right).length;
        setResults((prev) =>
          prev.map((g) =>
            g.leaf_id === l.id ? { ...g, status: "done", items: out } : g
          )
        );
      } catch (e: any) {
        setResults((prev) =>
          prev.map((g) =>
            g.leaf_id === l.id ? { ...g, status: "error", error: String(e) } : g
          )
        );
      }
    }
    setRunning(false);
    setPicked(new Set());
    if (totalQ > 0) {
      toast.success(`共 ${totalQ} 题，正确 ${totalOk} 题`);
    } else {
      toast.error("所有节点执行失败");
    }
  };

  return (
    <div>
      <SectionTitle
        title="自动作业"
        subtitle="批量勾选习题节点 → 拉取题目 → 询问大模型 → 自动提交。请先在设置中配置 OpenAI 兼容 API。"
      />
      <div className="px-12 flex gap-3 flex-wrap mb-4">
        {courses.map((c) => (
          <Capsule
            key={c.classroom_id}
            selected={selected?.classroom_id === c.classroom_id}
            onClick={() => loadLeaves(c)}
          >
            {c.name}
          </Capsule>
        ))}
      </div>
      {selected && (
        <div className="px-12 grid grid-cols-1 lg:grid-cols-3 gap-6 pb-8">
          <Card className="lg:col-span-2">
            <div className="flex items-center justify-between mb-3 flex-wrap gap-2">
              <div className="font-display text-tagline">
                习题节点
                <span className="ml-3 text-caption text-ink-muted-48 font-text">
                  共 {visibleLeaves.length} 个
                  {resolving && (
                    <span className="ml-2 anim-pulse">解析中…</span>
                  )}
                </span>
              </div>
              {(loading || resolving) && <Spinner />}
            </div>
            <div className="max-h-[420px] overflow-auto divide-y divide-divider-soft">
              {visibleLeaves.map((l) => {
                const ex = extra[l.id];
                const ready = !!ex?.exercise_id;
                return (
                  <label
                    key={l.id}
                    className={`flex items-center gap-3 py-2 transition-colors ${
                      ready
                        ? "cursor-pointer hover:bg-parchment/60 -mx-2 px-2 rounded-sm"
                        : "opacity-60 cursor-not-allowed"
                    }`}
                  >
                    <input
                      type="checkbox"
                      checked={picked.has(l.id)}
                      disabled={!ready || running}
                      onChange={() => toggle(l.id)}
                    />
                    <div className="flex-1 min-w-0">
                      <div className="text-body truncate">{l.name}</div>
                      <div className="text-fine text-ink-muted-48 truncate">
                        {l.chapter_path.join(" / ")} · 类型 {l.leaf_type}
                      </div>
                    </div>
                    <span className="text-fine text-ink-muted-48 min-w-[120px] text-right">
                      {ex?.exercise_id
                        ? `exercise: ${ex.exercise_id}`
                        : ex?.resolved
                        ? "无习题"
                        : "解析中…"}
                    </span>
                  </label>
                );
              })}
              {!loading && !resolving && visibleLeaves.length === 0 && (
                <div className="text-body text-ink-muted-80 py-6">
                  本课程无可执行的习题节点。
                </div>
              )}
            </div>
            <div className="flex items-center gap-3 mt-4 flex-wrap">
              <button
                className="text-link text-caption"
                onClick={selectAll}
                type="button"
              >
                全选
              </button>
              <button
                className="text-link text-caption"
                onClick={clearSelection}
                type="button"
              >
                清空
              </button>
              <div className="flex-1" />
              <Pill onClick={startAll} disabled={picked.size === 0 || running}>
                {running ? (
                  <span className="inline-flex items-center gap-2">
                    <Spinner /> 执行中…
                  </span>
                ) : (
                  `开始 ${picked.size} 个节点`
                )}
              </Pill>
            </div>
          </Card>
          <Card>
            <div className="font-display text-tagline mb-3">
              执行结果
              {results.length > 0 && (
                <span className="ml-2 text-caption text-ink-muted-48 font-text">
                  {results.filter((g) => g.status !== "running").length}/
                  {results.length}
                </span>
              )}
            </div>
            <div className="max-h-[460px] overflow-auto space-y-3">
              {results.map((g) => {
                const okCount = g.items.filter(
                  (r: any) => r?.submit?.is_right
                ).length;
                return (
                  <div
                    key={g.leaf_id}
                    className="border border-hairline rounded-md p-3 anim-in"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <div className="text-body text-ink truncate flex-1">
                        {g.leaf_name}
                      </div>
                      <span
                        className={`text-fine ${
                          g.status === "error"
                            ? "text-[#cc2b2b]"
                            : g.status === "running"
                            ? "text-action-blue anim-pulse"
                            : "text-action-blue"
                        }`}
                      >
                        {g.status === "running"
                          ? "执行中…"
                          : g.status === "error"
                          ? "失败"
                          : `${okCount}/${g.items.length} 正确`}
                      </span>
                    </div>
                    {g.error && (
                      <div className="text-fine text-[#cc2b2b] mt-1">
                        {g.error}
                      </div>
                    )}
                    {g.items.length > 0 && (
                      <div className="mt-2 space-y-1">
                        {g.items.map((r: any, i: number) => (
                          <div
                            key={i}
                            className={`text-fine ${
                              r.error || r.submit?.is_right === false
                                ? "text-[#cc2b2b]"
                                : "text-ink-muted-80"
                            }`}
                          >
                            题 {r.problem_id ?? "-"} · 答{" "}
                            {(r.answer || []).join("")}
                            {r.error ? ` · ${r.error}` : ""}
                            {r.submit
                              ? ` · ${
                                  r.submit.is_right ? "✓" : "✗"
                                } 得分 ${r.submit.my_score ?? "?"}`
                              : ""}
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })}
              {results.length === 0 && (
                <div className="text-body text-ink-muted-80">尚未执行。</div>
              )}
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}
