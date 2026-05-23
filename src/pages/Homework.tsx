import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  api,
  onBankProgress,
  onHomeworkProgress,
  type BankProgress,
  type Course,
  type HomeworkProgress,
  type LeafNode,
  type ProblemKind,
} from "../lib/api";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";
import { KindBadge } from "../components/KindBadge";
import { BookIcon, RefreshIcon } from "../components/icons";
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
  /**
   * 当前正在进行的步骤文本。改成 ReactNode 是为了让"题库命中"等场景能内嵌 BookIcon，
   * 同时保留普通字符串场景的写法（字符串本身就是合法的 ReactNode）。
   */
  currentLine?: ReactNode;
  /** 该节点本次执行从题库命中（跳过 AI）的题数 */
  bankHits?: number;
  /** 本次执行结束后自动入库的题数（来自学堂批改后再拉一次的结果） */
  bankHarvested?: number;
};

const kindLabel: Record<string, string> = {
  single_choice: "单选",
  multiple_choice: "多选",
  judgement: "判断",
  completion: "填空",
  subjective: "主观",
  other: "其它",
};

/**
 * 把单题结果拆成"是否带题库标记 + 主体文本"，避免在文本里直接拼 emoji。
 * 调用方在 JSX 里根据 fromBank 前置 <BookIcon /> 即可保持图文同色。
 */
function formatResultLine(r: any): { fromBank: boolean; text: string } {
  const score =
    r.submit && r.submit.is_right !== null && r.submit.is_right !== undefined
      ? ` · ${r.submit.is_right ? "✓" : "✗"} 得分 ${r.submit.my_score ?? "?"}`
      : "";
  if (r.skipped) {
    return { fromBank: false, text: `题 ${r.problem_id ?? "-"} · 已提交${score}` };
  }
  const answer = r.answer_text || (r.answer || []).join("") || "—";
  return {
    fromBank: !!r.from_bank,
    text: `题 ${r.problem_id ?? "-"} · 答 ${answer}${r.error ? ` · ${r.error}` : ""}${score}`,
  };
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
  // 单节点收录的"忙碌"集合：避免同一节点被重复点击触发并发请求
  const [harvesting, setHarvesting] = useState<Set<number>>(new Set());
  // 批量收录的进度（来自 bank://progress 事件流）
  const [harvestProgress, setHarvestProgress] = useState<{
    running: boolean;
    index: number;
    total: number;
    harvested: number;
    leafId?: number;
  } | null>(null);

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
            case "bank_hit": {
              const ans = (p.info?.answer_text ?? "").toString();
              const tail = ans.length > 12 ? `${ans.slice(0, 12)}…` : ans;
              return {
                ...g,
                bankHits: (g.bankHits ?? 0) + 1,
                currentLine: (
                  <span className="inline-flex items-center gap-1.5">
                    <span>题 {idx1}{g.total ? `/${g.total}` : ""} · </span>
                    <BookIcon className="w-3.5 h-3.5" />
                    <span>题库命中 {tail}</span>
                  </span>
                ),
              };
            }
            case "submitting": {
              const ans = (p.info?.answer_text ?? "").toString();
              const tail = ans.length > 12 ? `${ans.slice(0, 12)}…` : ans;
              if (p.info?.from_bank) {
                return {
                  ...g,
                  currentLine: (
                    <span className="inline-flex items-center gap-1.5">
                      <span>题 {idx1}{g.total ? `/${g.total}` : ""} · </span>
                      <BookIcon className="w-3.5 h-3.5" />
                      <span>正在提交 {tail}</span>
                    </span>
                  ),
                };
              }
              return {
                ...g,
                currentLine: `题 ${idx1}${g.total ? `/${g.total}` : ""} · 正在提交 ${tail}`,
              };
            }
            case "skipped":
              return { ...g, currentLine: `题 ${idx1}${g.total ? `/${g.total}` : ""} · 已批改，跳过` };
            case "delaying": {
              // 题库命中场景下的"节流等待"。AI 路径用 asking_ai 文案即可，不会进这里。
              const ms = p.info?.delay_ms ?? 0;
              const secs = (ms / 1000).toFixed(1);
              return {
                ...g,
                currentLine: (
                  <span className="inline-flex items-center gap-1.5">
                    <span>题 {idx1}{g.total ? `/${g.total}` : ""} · </span>
                    <BookIcon className="w-3.5 h-3.5" />
                    <span>节流 {secs}s 后提交</span>
                  </span>
                ),
              };
            }
            case "submit_failed": {
              // 第一次提交未被接受，正在等待重试。学堂限流(429)单独标注，方便用户判断网络环境。
              const ms = p.info?.retry_in_ms ?? 0;
              const secs = (ms / 1000).toFixed(1);
              const tag =
                p.info?.status === 429 || p.info?.submit_reason === "rate_limited"
                  ? "限流"
                  : "未被接受";
              return {
                ...g,
                currentLine: `题 ${idx1}${g.total ? `/${g.total}` : ""} · 提交${tag}，${secs}s 后重试…`,
              };
            }
            case "submit_retried": {
              // 第二次尝试已经结束；具体成功/失败的可视化由后续 item_done 渲染，这里只清提示。
              return { ...g, currentLine: undefined };
            }
            case "wrong_plan": {
              // 节点开始时上报本次控分计划。不改 currentLine（避免覆盖 start 文案），
              // 仅 toast 告知一次，让用户知道接下来会有 N 道题被故意答错。
              const n = p.info?.planned_wrong_count ?? 0;
              if (n > 0) {
                toast.info(`本节点将故意答错 ${n} 道题（控分）`);
              }
              return g;
            }
            case "intentional_wrong": {
              // 该题命中"故意答错"集合，已把题库/AI 的答案换成错答。
              // 这里只更新 currentLine，让用户看到"原本要答 X 但实际提交了 Y"。
              const orig = (p.info?.original_answer_text ?? "").toString();
              const wrong = (p.info?.wrong_answer_text ?? "").toString();
              const o = orig.length > 8 ? `${orig.slice(0, 8)}…` : orig;
              const w = wrong.length > 8 ? `${wrong.slice(0, 8)}…` : wrong;
              return {
                ...g,
                currentLine: `题 ${idx1}${g.total ? `/${g.total}` : ""} · 控分：${o} → 改提交 ${w}`,
              };
            }
            case "item_done": {
              const result = p.info?.result;
              return result ? { ...g, items: [result, ...g.items], currentLine: undefined } : g;
            }
            case "done": {
              // 节点结束：如果有题目两次都没被接受，统一弹 toast 告知用户。
              const failures = p.info?.failures;
              if (failures && failures.length > 0) {
                const lines = failures
                  .slice(0, 5)
                  .map((f) => `题 ${f.problem_id}：${f.error}`)
                  .join("\n");
                const more =
                  failures.length > 5 ? `\n…还有 ${failures.length - 5} 题失败` : "";
                toast.error(`本节点有 ${failures.length} 题提交失败：\n${lines}${more}`);
              }
              // 控分小结：实际故意答错的题数 > 0 时友情提示一下，便于用户事后核对成绩
              const wrongN = p.info?.intentional_wrong_count ?? 0;
              if (wrongN > 0 && !(failures && failures.length > 0)) {
                toast.success(`节点完成，按设置故意答错 ${wrongN} 题`);
              }
              return {
                ...g,
                currentLine: undefined,
                bankHarvested: p.info?.bank_harvested ?? g.bankHarvested,
              };
            }
          }
          return g;
        })
      );
    });
    return () => {
      unlistenPromise.then((fn) => fn()).catch(() => {});
    };
  }, []);

  // 监听批量收录的进度事件，更新顶部进度横条
  useEffect(() => {
    const unlistenPromise = onBankProgress((p: BankProgress) => {
      setHarvestProgress((prev) => {
        if (p.phase === "done") {
          return null;
        }
        const harvested = (p.extra?.total_harvested as number | undefined) ?? prev?.harvested ?? 0;
        return {
          running: true,
          index: p.index,
          total: p.total,
          harvested,
          leafId: (p.extra?.leaf_id as number | undefined) ?? prev?.leafId,
        };
      });
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

  /**
   * 单节点「收录答案」：仅对已完成的节点可用。
   * 调用学堂 get_exercise_list 一次，把响应里已批改的题入本地题库。
   * 不会发起任何提交，无副作用。
   */
  const harvestOne = async (l: LeafNode) => {
    if (!selected) return;
    const exId = extra[l.id]?.exercise_id;
    if (!exId) {
      toast.error("尚未解析到该节点的 exercise_id");
      return;
    }
    if (!isFinished(l.id)) {
      toast.error("仅已完成的节点能收录答案");
      return;
    }
    setHarvesting((s) => {
      const n = new Set(s);
      n.add(l.id);
      return n;
    });
    try {
      const r = await api.harvestExerciseAnswers({
        leaf_id: l.id,
        classroom_id: selected.classroom_id,
        sku_id: selected.sku_id,
        exercise_id: exId,
        sign: selected.sign,
      });
      if (r.harvested > 0) {
        toast.success(`已入库 ${r.harvested}/${r.submitted_problems} 题（共 ${r.total_problems}）`);
      } else if (r.submitted_problems === 0) {
        toast.info("该节点尚未有已批改的题目可收录");
      } else {
        toast.info(`无新增（共 ${r.submitted_problems} 道已批改题，可能已存在题库中）`);
      }
    } catch (e: any) {
      toast.error(`收录失败：${e}`);
    } finally {
      setHarvesting((s) => {
        const n = new Set(s);
        n.delete(l.id);
        return n;
      });
    }
  };

  /**
   * 批量「收录所有已完成节点的答案」。后端按 ≤ 1 req/s 节奏顺序处理。
   * 期间通过 `bank://progress` 实时上报，前端显示进度横条。
   */
  const harvestAllFinished = async () => {
    if (!selected) return;
    const targets = allHomeworkLeaves
      .filter((l) => extra[l.id]?.exercise_id && isFinished(l.id))
      .map((l) => ({ leaf_id: l.id, exercise_id: extra[l.id]!.exercise_id! }));
    if (targets.length === 0) {
      toast.info("没有可收录的已完成节点");
      return;
    }
    setHarvestProgress({ running: true, index: 0, total: targets.length, harvested: 0 });
    try {
      const r = await api.batchHarvestCourseAnswers({
        classroom_id: selected.classroom_id,
        sku_id: selected.sku_id,
        sign: selected.sign,
        leaves: targets,
      });
      if (r.total_harvested > 0) {
        toast.success(`批量收录完成：入库 ${r.total_harvested} 题 / 共 ${r.total_submitted} 道已批改`);
      } else {
        toast.info(`批量收录完成：无新增`);
      }
    } catch (e: any) {
      toast.error(`批量收录失败：${e}`);
    } finally {
      setHarvestProgress(null);
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
                <button
                  className="text-link text-caption inline-flex items-center gap-1"
                  onClick={refreshSchedule}
                  title="重新拉取课程节点的完成进度"
                >
                  <RefreshIcon className="w-3.5 h-3.5" />
                  刷新进度
                </button>
                <button
                  type="button"
                  className="text-link text-caption disabled:text-ink-muted-48 disabled:no-underline inline-flex items-center gap-1"
                  onClick={harvestAllFinished}
                  disabled={
                    !!harvestProgress ||
                    finishedCount === 0 ||
                    running
                  }
                  title="把所有已完成节点的标准答案拉到本地题库"
                >
                  <BookIcon className="w-3.5 h-3.5" />
                  收录已完成节点
                </button>
                <label className="text-caption text-ink-muted-80 inline-flex items-center gap-1">
                  <input type="checkbox" checked={hideFinished} onChange={(e) => setHideFinished(e.target.checked)} />
                  仅显示未完成
                </label>
              </div>
            </div>
            {harvestProgress && (
              <div className="mb-3 px-3 py-2 rounded-card bg-action-blue/5 text-caption text-action-blue flex items-center gap-2">
                <Spinner />
                <span>
                  批量收录中… {harvestProgress.index + 1}/{harvestProgress.total}
                  {harvestProgress.harvested > 0 ? ` · 已入库 ${harvestProgress.harvested} 题` : ""}
                </span>
              </div>
            )}
            <div className="max-h-[420px] overflow-auto divide-y divide-divider-soft">
              {visibleLeaves.map((l) => {
                const ex = extra[l.id];
                const ready = !!ex?.exercise_id;
                const finished = isFinished(l.id);
                const disabled = !ready || running || finished;
                const isHarvesting = harvesting.has(l.id);
                return (
                  <label key={l.id} className={`flex items-start gap-3 py-2 transition-colors ${disabled ? "opacity-60" : ""} ${disabled ? "cursor-not-allowed" : "cursor-pointer hover:bg-parchment/60 -mx-2 px-2 rounded-sm"}`}>
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
                    <div className="flex flex-col items-end gap-1 min-w-[110px]">
                      <span className="text-fine text-ink-muted-48 text-right pt-0.5">
                        {ex?.exercise_id ? `exercise: ${ex.exercise_id}` : ex?.resolved ? "无习题" : "解析中…"}
                      </span>
                      {finished && ready && (
                        <button
                          type="button"
                          className="text-fine text-link disabled:text-ink-muted-48 disabled:no-underline inline-flex items-center gap-1"
                          onClick={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            harvestOne(l);
                          }}
                          disabled={isHarvesting || !!harvestProgress}
                          title="把该节点的标准答案拉到本地题库（不会重新提交）"
                        >
                          {isHarvesting ? (
                            <>
                              <Spinner /> 收录中
                            </>
                          ) : (
                            <>
                              <BookIcon className="w-3 h-3" />
                              收录答案
                            </>
                          )}
                        </button>
                      )}
                    </div>
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
                const bankCount = g.items.filter((r: any) => r?.from_bank).length;
                const resultTotal = g.total ?? g.items.length;
                const latestResult = g.items[0];
                const showDoneLine = g.status === "done";
                return (
                  <div key={g.leaf_id} className="border border-divider-soft rounded-card p-3 bg-white/50 anim-in">
                    <div className="text-body truncate">{g.leaf_name}</div>
                    <div className={`text-caption mt-1 ${g.status === "error" ? "text-[#cc2b2b]" : g.status === "running" ? "text-action-blue anim-pulse" : "text-action-blue"}`}>
                      {g.status === "running" ? `${okCount}/${resultTotal || "?"} 正确` : g.status === "error" ? "失败" : `${okCount}/${g.items.length} 正确`}
                      {bankCount > 0 && (
                        <span className="ml-2 text-ink-muted-80 inline-flex items-center gap-1 align-middle">
                          <BookIcon className="w-3 h-3" />
                          命中 {bankCount}
                        </span>
                      )}
                      {g.bankHarvested != null && g.bankHarvested > 0 && (
                        <span className="ml-2 text-ink-muted-80">入库 {g.bankHarvested}</span>
                      )}
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
                              {(() => {
                                const r = formatResultLine(latestResult);
                                return (
                                  <span className="flex-1 min-w-0 whitespace-normal break-words leading-relaxed inline-flex flex-wrap items-baseline gap-1">
                                    {r.fromBank && (
                                      <BookIcon className="w-3 h-3 shrink-0 self-center" />
                                    )}
                                    <span>{r.text}</span>
                                  </span>
                                );
                              })()}
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
