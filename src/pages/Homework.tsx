import { useEffect, useMemo, useState } from "react";
import { api, type Course, type LeafNode, type ProblemKind } from "../lib/api";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";
import { KindBadge } from "../components/KindBadge";
import { toast } from "../components/Toast";

type LeafExtra = {
  exercise_id?: number;
  resolved?: boolean; // 已尝试解析（无论成功或失败）
  kinds?: Record<string, number>; // 题型计数
  total?: number;
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
  const [schedule, setSchedule] = useState<Record<string, number>>({});
  const [loading, setLoading] = useState(false);
  const [resolving, setResolving] = useState(false);
  const [running, setRunning] = useState(false);
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [results, setResults] = useState<ResultGroup[]>([]);
  const [hideFinished, setHideFinished] = useState(false);

  const allHomeworkLeaves = useMemo(
    () => leaves.filter((l) => l.leaf_type === 6 || l.leaf_type === 7 || l.leaf_type === 3),
    [leaves]
  );

  const isFinished = (id: number) => (schedule[String(id)] ?? 0) >= 1;

  // 仅展示真正有 exercise_id 的节点；尚未解析完成的节点暂保留以显示"解析中"
  const visibleLeaves = useMemo(
    () =>
      allHomeworkLeaves.filter((l) => {
        if (hideFinished && isFinished(l.id)) return false;
        const ex = extra[l.id];
        if (!ex) return true;
        if (!ex.resolved) return true;
        return ex.exercise_id != null && ex.exercise_id > 0;
      }),
    [allHomeworkLeaves, extra, schedule, hideFinished]
  );

  const finishedCount = useMemo(
    () =>
      allHomeworkLeaves.filter(
        (l) => extra[l.id]?.exercise_id && isFinished(l.id)
      ).length,
    [allHomeworkLeaves, extra, schedule]
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
      const [ls, sched] = await Promise.all([
        api.listChapters(c.classroom_id, c.sign),
        api.courseSchedule(c.classroom_id, c.sign).catch(() => ({})),
      ]);
      setLeaves(ls);
      setSchedule(sched);

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
        const validItems: [number, number][] = [];
        for (const l of hwLeafs) {
          const ex = (map as any)[String(l.id)] ?? (map as any)[l.id];
          const exId = typeof ex === "number" ? ex : undefined;
          next[l.id] = { exercise_id: exId, resolved: true };
          if (exId) validItems.push([l.id, exId]);
        }
        setExtra(next);

        // 第二阶段：异步并行拉每个习题集的题型分布
        if (validItems.length > 0) {
          api
            .batchExerciseKinds(c.sku_id, validItems)
            .then((kindsMap) => {
              setExtra((prev) => {
                const updated = { ...prev };
                for (const [leafId] of validItems) {
                  const kinds =
                    (kindsMap as any)[String(leafId)] ??
                    (kindsMap as any)[leafId] ??
                    {};
                  const total = Object.values(kinds).reduce(
                    (a: number, b: any) => a + Number(b),
                    0
                  );
                  updated[leafId] = {
                    ...updated[leafId],
                    kinds,
                    total,
                  };
                }
                return updated;
              });
            })
            .catch((e) => {
              // 题型预览失败不阻塞主流程
              console.warn("批量拉题型失败", e);
            });
        }
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
    if (isFinished(id)) return;
    const n = new Set(picked);
    if (n.has(id)) n.delete(id);
    else n.add(id);
    setPicked(n);
  };

  const selectAll = () => {
    setPicked(
      new Set(
        visibleLeaves
          .filter((l) => extra[l.id]?.exercise_id && !isFinished(l.id))
          .map((l) => l.id)
      )
    );
  };

  const clearSelection = () => setPicked(new Set());

  const refreshSchedule = async () => {
    if (!selected) return;
    try {
      setSchedule(
        await api.courseSchedule(selected.classroom_id, selected.sign)
      );
      toast.info("进度已刷新");
    } catch (e: any) {
      toast.error(String(e));
    }
  };

  const startAll = async () => {
    if (!selected || picked.size === 0) return;
    setRunning(true);
    const targets = visibleLeaves.filter(
      (l) => picked.has(l.id) && extra[l.id]?.exercise_id && !isFinished(l.id)
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
        // 乐观把该 leaf 标记为已完成（rate=1）
        setSchedule((s) => ({ ...s, [String(l.id)]: 1 }));
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
    // 拉一次真实进度兜底
    if (selected) {
      api
        .courseSchedule(selected.classroom_id, selected.sign)
        .then(setSchedule)
        .catch(() => {});
    }
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
                  共 {allHomeworkLeaves.filter((l) => extra[l.id]?.exercise_id).length} 个，已完成 {finishedCount}
                  {resolving && (
                    <span className="ml-2 anim-pulse">解析中…</span>
                  )}
                </span>
              </div>
              <div className="flex items-center gap-3">
                {(loading || resolving) && <Spinner />}
                <button
                  className="text-link text-caption"
                  onClick={refreshSchedule}
                >
                  刷新进度
                </button>
                <label className="text-caption text-ink-muted-80 inline-flex items-center gap-1">
                  <input
                    type="checkbox"
                    checked={hideFinished}
                    onChange={(e) => setHideFinished(e.target.checked)}
                  />
                  仅显示未完成
                </label>
              </div>
            </div>
            <div className="max-h-[420px] overflow-auto divide-y divide-divider-soft">
              {visibleLeaves.map((l) => {
                const ex = extra[l.id];
                const ready = !!ex?.exercise_id;
                const finished = isFinished(l.id);
                const disabled = !ready || running || finished;
                return (
                  <label
                    key={l.id}
                    className={`flex items-start gap-3 py-2 transition-colors ${
                      disabled
                        ? "opacity-60 cursor-not-allowed"
                        : "cursor-pointer hover:bg-parchment/60 -mx-2 px-2 rounded-sm"
                    }`}
                  >
                    <input
                      type="checkbox"
                      checked={picked.has(l.id)}
                      disabled={disabled}
                      onChange={() => toggle(l.id)}
                      className="mt-1.5"
                    />
                    <div className="flex-1 min-w-0">
                      <div className="text-body truncate flex items-center gap-2">
                        <span className="truncate">{l.name}</span>
                        {finished && (
                          <span className="inline-flex items-center text-fine leading-none h-[20px] px-2 text-action-blue bg-action-blue/10 rounded-pill whitespace-nowrap">
                            已完成
                          </span>
                        )}
                      </div>
                      <div className="text-fine text-ink-muted-48 truncate">
                        {l.chapter_path.join(" / ")} · 类型 {l.leaf_type}
                      </div>
                      {/* 题型分布徽章 —— 与"共 X 题"在同一行，使用相同高度对齐 */}
                      {ex?.kinds && ex.total ? (
                        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                          <span className="text-fine text-ink-muted-48 leading-none h-[20px] inline-flex items-center">
                            共 {ex.total} 题
                          </span>
                          {Object.entries(ex.kinds)
                            .sort((a, b) => b[1] - a[1])
                            .map(([k, n]) => (
                              <KindBadge
                                key={k}
                                kind={k as any}
                                text={`${
                                  ({
                                    single_choice: "单选",
                                    multiple_choice: "多选",
                                    judgement: "判断",
                                    completion: "填空",
                                    subjective: "主观",
                                    other: "其它",
                                  } as any)[k] ?? k
                                } ${n}`}
                              />
                            ))}
                        </div>
                      ) : ready ? (
                        <div className="mt-1 text-fine text-ink-muted-48 anim-pulse">
                          正在解析题型…
                        </div>
                      ) : null}
                    </div>
                    <span className="text-fine text-ink-muted-48 min-w-[110px] text-right pt-0.5">
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
                  {hideFinished && allHomeworkLeaves.length > 0
                    ? "所有习题均已完成。"
                    : "本课程无可执行的习题节点。"}
                </div>
              )}
            </div>
            <div className="flex items-center gap-3 mt-4 flex-wrap">
              <button
                className="text-link text-caption disabled:text-ink-muted-48 disabled:no-underline"
                onClick={selectAll}
                disabled={running}
                type="button"
              >
                全选未完成
              </button>
              <button
                className="text-link text-caption disabled:text-ink-muted-48 disabled:no-underline"
                onClick={clearSelection}
                disabled={running}
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
                      <>
                        {/* 题型分布统计 */}
                        <div className="mt-2 flex gap-1.5 flex-wrap">
                          {Object.entries(
                            g.items.reduce<Record<string, number>>((acc, r: any) => {
                              const k = (r.kind as ProblemKind) ?? "other";
                              acc[k] = (acc[k] ?? 0) + 1;
                              return acc;
                            }, {})
                          ).map(([k, n]) => (
                            <KindBadge
                              key={k}
                              kind={k as ProblemKind}
                              text={`${
                                {
                                  single_choice: "单选",
                                  multiple_choice: "多选",
                                  judgement: "判断",
                                  completion: "填空",
                                  subjective: "主观",
                                  other: "其它",
                                }[k as ProblemKind] ?? k
                              } × ${n}`}
                            />
                          ))}
                        </div>
                        <div className="mt-2 space-y-1">
                          {g.items.map((r: any, i: number) => (
                            <div
                              key={i}
                              className={`text-fine flex items-center gap-2 ${
                                r.error || r.submit?.is_right === false
                                  ? "text-[#cc2b2b]"
                                  : "text-ink-muted-80"
                              }`}
                            >
                              <KindBadge kind={r.kind as ProblemKind} />
                              <span className="flex-1 truncate">
                                题 {r.problem_id ?? "-"} · 答{" "}
                                {r.answer_text ||
                                  (r.answer || []).join("") ||
                                  "—"}
                                {r.error ? ` · ${r.error}` : ""}
                                {r.submit
                                  ? ` · ${
                                      r.submit.is_right ? "✓" : "✗"
                                    } 得分 ${r.submit.my_score ?? "?"}`
                                  : ""}
                              </span>
                            </div>
                          ))}
                        </div>
                      </>
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
