import { useEffect, useMemo, useState } from "react";
import {
  api,
  onHomeworkProgress,
  type Course,
  type HomeworkProgress,
  type LeafNode,
  type ProblemKind,
} from "../lib/api";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";
import { KindBadge } from "../components/KindBadge";
import { toast } from "../components/Toast";

type LeafExtra = {
  exercise_id?: number;
  resolved?: boolean;
  kinds?: Record<string, number>;
  total?: number;
};

type ResultGroup = {
  leaf_id: number;
  leaf_name: string;
  status: "running" | "done" | "error";
  error?: string;
  items: any[];
  total?: number;
  currentLine?: string;
};

const kindLabel: Record<string, string> = {
  single_choice: "单选",
  multiple_choice: "多选",
  judgement: "判断",
  completion: "填空",
  subjective: "主观",
  other: "其它",
};

function formatResultLine(r: any): string {
  const score =
    r.submit && r.submit.is_right !== null && r.submit.is_right !== undefined
      ? ` · ${r.submit.is_right ? "✓" : "✗"} 得分 ${r.submit.my_score ?? "?"}`
      : "";
  if (r.skipped) {
    return `题 ${r.problem_id ?? "-"} · 已提交${score}`;
  }
  const answer = r.answer_text || (r.answer || []).join("") || "—";
  return `题 ${r.problem_id ?? "-"} · 答 ${answer}${r.error ? ` · ${r.error}` : ""}${score}`;
}

function parseCaptchaRequired(err: unknown): { appId: string } | null {
  const text = String(err ?? "");
  const m = text.match(/CAPTCHA_REQUIRED:([^:]+):/);
  return m ? { appId: m[1] } : null;
}

function runTencentCaptcha(appId: string): Promise<{ ticket: string; randstr: string }> {
  return new Promise((resolve, reject) => {
    const Ctor = window.TencentCaptcha;
    if (!Ctor) {
      reject(new Error("腾讯滑块脚本未加载，请检查网络后重试"));
      return;
    }
    const captcha = new Ctor(appId, (res) => {
      if (res.ret === 0 && res.ticket && res.randstr) {
        resolve({ ticket: res.ticket, randstr: res.randstr });
      } else {
        reject(new Error(res.errorMessage || "已取消滑块验证"));
      }
    });
    captcha.show();
  });
}

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
    () => allHomeworkLeaves.filter((l) => extra[l.id]?.exercise_id && isFinished(l.id)).length,
    [allHomeworkLeaves, extra, schedule]
  );

  useEffect(() => {
    api.listCourses().then(setCourses);
  }, []);

  useEffect(() => {
    const unlistenPromise = onHomeworkProgress((p: HomeworkProgress) => {
      const idx1 = (p.info?.index ?? 0) + 1;
      const total = p.info?.total;
      const label = p.info?.kind_label ?? "";
      setResults((prev) =>
        prev.map((g) => {
          if (g.leaf_id !== p.leaf_id) return g;
          switch (p.phase) {
            case "start":
              return { ...g, total: total ?? g.total, currentLine: `题 ${idx1}/${total ?? g.total ?? "?"} · ${label}` };
            case "asking_ai":
              return { ...g, currentLine: `题 ${idx1}${g.total ? `/${g.total}` : ""} · 正在询问 AI…` };
            case "submitting": {
              const ans = (p.info?.answer_text ?? "").toString();
              return { ...g, currentLine: `题 ${idx1}${g.total ? `/${g.total}` : ""} · 正在提交 ${ans.length > 12 ? `${ans.slice(0, 12)}…` : ans}` };
            }
            case "skipped":
              return { ...g, currentLine: `题 ${idx1}${g.total ? `/${g.total}` : ""} · 已批改，跳过` };
            case "item_done": {
              const result = p.info?.result;
              return result ? { ...g, items: [result, ...g.items], currentLine: undefined } : g;
            }
            case "done":
              return { ...g, currentLine: undefined };
          }
          return g;
        })
      );
    });
    return () => {
      unlistenPromise.then((fn) => fn()).catch(() => {});
    };
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

      const hwLeafs = ls.filter((x) => x.leaf_type === 6 || x.leaf_type === 7 || x.leaf_type === 3);
      if (hwLeafs.length === 0) return;

      setResolving(true);
      try {
        const map = await api.batchExerciseIds(c.classroom_id, c.sign, hwLeafs.map((l) => l.id));
        const next: Record<number, LeafExtra> = {};
        for (const l of hwLeafs) {
          const ex = (map as any)[String(l.id)] ?? (map as any)[l.id];
          const exId = typeof ex === "number" ? ex : undefined;
          next[l.id] = { exercise_id: exId, resolved: true };
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
    if (isFinished(id)) return;
    const n = new Set(picked);
    if (n.has(id)) n.delete(id);
    else n.add(id);
    setPicked(n);
  };

  const selectAll = () => {
    setPicked(new Set(visibleLeaves.filter((l) => extra[l.id]?.exercise_id && !isFinished(l.id)).map((l) => l.id)));
  };

  const clearSelection = () => setPicked(new Set());

  const refreshSchedule = async () => {
    if (!selected) return;
    try {
      setSchedule(await api.courseSchedule(selected.classroom_id, selected.sign));
      toast.info("进度已刷新");
    } catch (e: any) {
      toast.error(String(e));
    }
  };

  const startAll = async () => {
    if (!selected || picked.size === 0) return;
    setRunning(true);
    const targets = visibleLeaves.filter((l) => picked.has(l.id) && extra[l.id]?.exercise_id && !isFinished(l.id));
    setResults(targets.map((l) => ({ leaf_id: l.id, leaf_name: l.name, status: "running", items: [] })));
    toast.info(`已提交 ${targets.length} 个节点的作业`);

    let totalOk = 0;
    let totalQ = 0;
    for (const l of targets) {
      try {
        const baseArgs = {
          leaf_id: l.id,
          classroom_id: selected.classroom_id,
          sku_id: selected.sku_id,
          exercise_id: extra[l.id]!.exercise_id!,
          sign: selected.sign,
        };
        let out: any[];
        try {
          out = await api.autoHomeworkLeaf(baseArgs);
        } catch (firstErr: any) {
          const captcha = parseCaptchaRequired(firstErr);
          if (!captcha) throw firstErr;
          toast.info("学堂触发滑块风控，请完成验证后自动继续");
          const solved = await runTencentCaptcha(captcha.appId);
          out = await api.autoHomeworkLeaf({ ...baseArgs, ...solved });
        }
        totalQ += out.length;
        totalOk += out.filter((r: any) => r?.submit?.is_right).length;
        setResults((prev) =>
          prev.map((g) => (g.leaf_id === l.id ? { ...g, status: "done", items: g.items.length === out.length ? g.items : [...out].reverse(), currentLine: undefined } : g))
        );
        setSchedule((s) => ({ ...s, [String(l.id)]: 1 }));
      } catch (e: any) {
        setResults((prev) => prev.map((g) => (g.leaf_id === l.id ? { ...g, status: "error", error: String(e), currentLine: undefined } : g)));
      }
    }
    setRunning(false);
    setPicked(new Set());
    if (selected) api.courseSchedule(selected.classroom_id, selected.sign).then(setSchedule).catch(() => {});
    if (totalQ > 0) toast.success(`共 ${totalQ} 题，正确 ${totalOk} 题`);
    else toast.error("所有节点执行失败");
  };

  return (
    <div>
      <SectionTitle title="自动作业" subtitle="批量勾选习题节点 → 拉取题目 → 询问大模型 → 自动提交。触发风控时会弹出滑块，验证后自动继续。" />
      <div className="px-12 flex gap-3 flex-wrap mb-4">
        {courses.map((c) => (
          <Capsule key={c.classroom_id} selected={selected?.classroom_id === c.classroom_id} onClick={() => loadLeaves(c)}>
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
                  {resolving && <span className="ml-2 anim-pulse">解析中…</span>}
                </span>
              </div>
              <div className="flex items-center gap-3">
                {(loading || resolving) && <Spinner />}
                <button className="text-link text-caption" onClick={refreshSchedule}>刷新进度</button>
                <label className="text-caption text-ink-muted-80 inline-flex items-center gap-1">
                  <input type="checkbox" checked={hideFinished} onChange={(e) => setHideFinished(e.target.checked)} />
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
                  <label key={l.id} className={`flex items-start gap-3 py-2 transition-colors ${disabled ? "opacity-60 cursor-not-allowed" : "cursor-pointer hover:bg-parchment/60 -mx-2 px-2 rounded-sm"}`}>
                    <input type="checkbox" checked={picked.has(l.id)} disabled={disabled} onChange={() => toggle(l.id)} className="mt-1.5" />
                    <div className="flex-1 min-w-0">
                      <div className="text-body truncate flex items-center gap-2">
                        <span className="truncate">{l.name}</span>
                        {finished && <span className="inline-flex items-center text-fine leading-none h-[20px] px-2 text-action-blue bg-action-blue/10 rounded-pill whitespace-nowrap">已完成</span>}
                      </div>
                      <div className="text-fine text-ink-muted-48 truncate">{l.chapter_path.join(" / ")} · 类型 {l.leaf_type}</div>
                      {ex?.kinds && ex.total ? (
                        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                          <span className="text-fine text-ink-muted-48 leading-none h-[20px] inline-flex items-center">共 {ex.total} 题</span>
                          {Object.entries(ex.kinds).sort((a, b) => b[1] - a[1]).map(([k, n]) => (
                            <KindBadge key={k} kind={k as any} text={`${kindLabel[k] ?? k} ${n}`} />
                          ))}
                        </div>
                      ) : null}
                    </div>
                    <span className="text-fine text-ink-muted-48 min-w-[110px] text-right pt-0.5">
                      {ex?.exercise_id ? `exercise: ${ex.exercise_id}` : ex?.resolved ? "无习题" : "解析中…"}
                    </span>
                  </label>
                );
              })}
              {!loading && !resolving && visibleLeaves.length === 0 && (
                <div className="text-body text-ink-muted-80 py-6">{hideFinished && allHomeworkLeaves.length > 0 ? "所有习题均已完成。" : "本课程无可执行的习题节点。"}</div>
              )}
            </div>
            <div className="flex items-center gap-3 mt-4 flex-wrap">
              <button className="text-link text-caption disabled:text-ink-muted-48 disabled:no-underline" onClick={selectAll} disabled={running} type="button">全选未完成</button>
              <button className="text-link text-caption disabled:text-ink-muted-48 disabled:no-underline" onClick={clearSelection} disabled={running} type="button">清空</button>
              <div className="flex-1" />
              <Pill onClick={startAll} disabled={picked.size === 0 || running}>{running ? <span className="inline-flex items-center gap-2"><Spinner /> 执行中…</span> : `开始 ${picked.size} 个节点`}</Pill>
            </div>
          </Card>
          <Card>
            <div className="font-display text-tagline mb-3">
              执行结果
              {results.length > 0 && <span className="ml-2 text-caption text-ink-muted-48 font-text">{results.filter((g) => g.status !== "running").length}/{results.length}</span>}
            </div>
            <div className="max-h-[460px] overflow-auto space-y-3">
              {results.map((g) => {
                const okCount = g.items.filter((r: any) => r?.submit?.is_right).length;
                const resultTotal = g.total ?? g.items.length;
                const latestResult = g.items[0];
                const showDoneLine = g.status === "done";
                return (
                  <div key={g.leaf_id} className="border border-divider-soft rounded-card p-3 bg-white/50 anim-in">
                    <div className="text-body truncate">{g.leaf_name}</div>
                    <div className={`text-caption mt-1 ${g.status === "error" ? "text-[#cc2b2b]" : g.status === "running" ? "text-action-blue anim-pulse" : "text-action-blue"}`}>
                      {g.status === "running" ? `${okCount}/${resultTotal || "?"} 正确` : g.status === "error" ? "失败" : `${okCount}/${g.items.length} 正确`}
                    </div>
                    {(g.items.length > 0 || showDoneLine) && (
                      <>
                        {g.items.length > 0 && <div className="mt-2 flex gap-1.5 flex-wrap">
                          {Object.entries(g.items.reduce<Record<string, number>>((acc, r: any) => {
                            const k = (r.kind as ProblemKind) ?? "other";
                            acc[k] = (acc[k] ?? 0) + 1;
                            return acc;
                          }, {})).map(([k, n]) => <KindBadge key={k} kind={k as ProblemKind} text={`${kindLabel[k] ?? k} × ${n}`} />)}
                        </div>}
                        <div className="mt-2 overflow-hidden">
                          {showDoneLine ? (
                            <div key={`done-${g.leaf_id}-${g.items.length}`} className="text-fine flex items-start gap-2 anim-rise text-[#159e55]">
                              <span className="inline-flex items-center text-fine leading-none h-[20px] px-2 text-[#159e55] bg-[#159e55]/10 rounded-pill whitespace-nowrap">完成</span>
                              <span className="flex-1 min-w-0 whitespace-normal break-words leading-relaxed">任务已完成</span>
                            </div>
                          ) : latestResult ? (
                            <div key={`${latestResult.problem_id ?? "unknown"}-${g.items.length}`} className={`text-fine flex items-start gap-2 anim-rise ${latestResult.skipped ? "text-ink-muted-48" : latestResult.error || latestResult.submit?.is_right === false ? "text-[#cc2b2b]" : "text-ink-muted-80"}`}>
                              <KindBadge kind={latestResult.kind as ProblemKind} />
                              <span className="flex-1 min-w-0 whitespace-normal break-words leading-relaxed">{formatResultLine(latestResult)}</span>
                            </div>
                          ) : null}
                        </div>
                      </>
                    )}
                  </div>
                );
              })}
              {results.length === 0 && <div className="text-body text-ink-muted-80">尚未执行。</div>}
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}
