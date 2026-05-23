import { useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  onArticleProgress,
  onForumProgress,
  type ArticleProgress,
  type Course,
  type ForumProgress,
  type LeafNode,
} from "../lib/api";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";
import { toast } from "../components/Toast";

/**
 * 任务类型定义。
 *
 * 学堂在线把不同 leaf_type 的节点都丢进同一 forum 接口体系，但行为差异巨大：
 * - 视频节点（leaf_type=0）：底下的讨论是"附属讨论区"，topic_type=0，没有分数。
 * - 讨论节点（leaf_type=4，独立节点）：topic_type=4，发评论后 POST chapter/schedule 才记分。
 * - 图文节点（leaf_type=3）：完全不发评论，只需要 POST chapter/schedule 即可标记已读。
 *
 * 这里用统一的 `TaskMode` 让 UI / 调用栈分支清晰：
 * - leafFilter 用于从章节树中筛出对应类型的节点
 * - requiresText 决定是否展示"评论文本"输入
 * - topicType / reportSchedule 透传给后端 auto_comment_leaf
 */
type TaskMode = "video_comment" | "scored_discussion" | "article";

const TASK_META: Record<
  TaskMode,
  {
    label: string;
    subtitle: string;
    leafType: number;
    requiresText: boolean;
    topicType?: number;
    reportSchedule?: boolean;
  }
> = {
  video_comment: {
    label: "视频讨论",
    subtitle: "在视频节点底下的讨论区批量发同一条评论（不带分数）。",
    leafType: 0,
    requiresText: true,
    topicType: 0,
    reportSchedule: false,
  },
  scored_discussion: {
    label: "讨论（带分加）",
    subtitle:
      "针对带分数的独立讨论节点（leaf_type=4）：发评论后顺手上报 chapter/schedule，触发该节点的计分。",
    leafType: 4,
    requiresText: true,
    topicType: 4,
    reportSchedule: true,
  },
  article: {
    label: "图文",
    subtitle: "标记图文节点（leaf_type=3）为已学完，仅调用 chapter/schedule。",
    leafType: 3,
    requiresText: false,
  },
};

const TASK_ORDER: TaskMode[] = ["video_comment", "scored_discussion", "article"];

/** 任意一种"批量任务"页都共享的进度行（限速/重试/完成）。 */
type ProgressMessage = string;

export function ForumPage() {
  const [mode, setMode] = useState<TaskMode>("video_comment");
  const [courses, setCourses] = useState<Course[]>([]);
  const [selected, setSelected] = useState<Course | null>(null);
  const [leaves, setLeaves] = useState<LeafNode[]>([]);
  const [loading, setLoading] = useState(false);
  const [text, setText] = useState("");
  // 不同 mode 下的勾选 / 结果 / 完成状态分别持有，切 tab 不互相干扰。
  // key = leaf_id；commentedMap 仅对评论类 mode 有意义，articleFinishedMap 仅对图文 mode。
  const [pickedByMode, setPickedByMode] = useState<Record<TaskMode, Set<number>>>(() => ({
    video_comment: new Set(),
    scored_discussion: new Set(),
    article: new Set(),
  }));
  const [resultsByMode, setResultsByMode] = useState<Record<TaskMode, any[]>>(() => ({
    video_comment: [],
    scored_discussion: [],
    article: [],
  }));
  const [running, setRunning] = useState(false);
  const [progressLine, setProgressLine] = useState<ProgressMessage>("");
  const countdownEnd = useRef<number | null>(null);

  /** 评论已发状态：mode -> { leaf_id: true }。仅评论类 mode 使用。 */
  const [commentedByMode, setCommentedByMode] = useState<
    Record<TaskMode, Record<number, boolean>>
  >(() => ({
    video_comment: {},
    scored_discussion: {},
    article: {},
  }));
  /**
   * 题目标题：mode -> { leaf_id: title }。
   *
   * 由后端 `batch_my_comment_status` 一起返回（从 forum/unit/discussion 的
   * content.text 提取）。仅评论类 mode 使用。所有"讨论（带分加）"节点在章节树
   * 里 leaf.name 都叫"案例分析"，没有这个补充标题就完全无法区分（即用户说的
   * "张冠李戴"）。
   */
  const [topicTitleByMode, setTopicTitleByMode] = useState<
    Record<TaskMode, Record<number, string>>
  >(() => ({
    video_comment: {},
    scored_discussion: {},
    article: {},
  }));
  /** 图文已学状态：通过 course/schedule 拉得到的 leaf -> rate >=1。 */
  const [articleFinished, setArticleFinished] = useState<Record<number, boolean>>(
    {}
  );
  const [probing, setProbing] = useState(false);

  useEffect(() => {
    api.listCourses().then(setCourses);
    api.getSettings().then((s) => setText(s.auto_comment_default || ""));
  }, []);

  // 订阅评论批量进度事件。切 tab 不会重新挂载（App 用 hidden），所以这里只在
  // 组件初次 mount 时注册一次。已评论 mode 的进度统一渲染到 progressLine。
  useEffect(() => {
    const unlistenPromise = onForumProgress((p: ForumProgress) => {
      switch (p.phase) {
        case "throttle": {
          const sec = Math.ceil(((p.extra?.wait_ms as number) ?? 0) / 1000);
          countdownEnd.current = null;
          setProgressLine(
            `第 ${p.index + 1}/${p.total} 条 · 限流间隔，等待 ${sec} 秒`
          );
          break;
        }
        case "sending": {
          const attempt = (p.extra?.attempt as number) ?? 0;
          countdownEnd.current = null;
          setProgressLine(
            `正在发送第 ${p.index + 1}/${p.total} 条${
              attempt > 0 ? ` · 第 ${attempt + 1} 次尝试` : ""
            }`
          );
          break;
        }
        case "rate_limited": {
          const ra = (p.extra?.retry_after_s as number) ?? 0;
          const attempt = (p.extra?.attempt as number) ?? 1;
          countdownEnd.current = Date.now() + ra * 1000;
          setProgressLine(
            `第 ${p.index + 1}/${p.total} 条命中限速，等待 ${Math.ceil(ra)} 秒后第 ${attempt + 1} 次重试…`
          );
          break;
        }
        case "item": {
          const result = p.extra as any;
          if (result && typeof result === "object" && "leaf_id" in result) {
            // 把结果追加到当前任务模式的 results。注意必须用函数式 setState
            // 读到最新的 mode（modeRef 不必，因为 setState 内部 read modeRef
            // 较为复杂；这里改用 modeRef 保存当前 active mode 即可）。
            // 为了简单起见，使用 modeRef.current。
            const m = modeRef.current;
            setResultsByMode((prev) => ({
              ...prev,
              [m]: [...prev[m], result],
            }));
            if (result.ok) {
              const lid = Number(result.leaf_id);
              setCommentedByMode((prev) => {
                if (prev[m][lid]) return prev;
                return { ...prev, [m]: { ...prev[m], [lid]: true } };
              });
            }
          }
          break;
        }
        case "done":
          countdownEnd.current = null;
          setProgressLine("");
          break;
      }
    });

    const unlistenArticlePromise = onArticleProgress((p: ArticleProgress) => {
      switch (p.phase) {
        case "throttle": {
          const sec = Math.ceil(((p.extra?.wait_ms as number) ?? 0) / 1000);
          setProgressLine(
            `第 ${p.index + 1}/${p.total} 条 · 节流间隔，等待 ${sec} 秒`
          );
          break;
        }
        case "sending": {
          setProgressLine(`正在标记第 ${p.index + 1}/${p.total} 条…`);
          break;
        }
        case "item": {
          const result = p.extra as any;
          if (result && typeof result === "object" && "leaf_id" in result) {
            setResultsByMode((prev) => ({
              ...prev,
              article: [...prev.article, result],
            }));
            if (result.ok) {
              const lid = Number(result.leaf_id);
              setArticleFinished((m) => (m[lid] ? m : { ...m, [lid]: true }));
            }
          }
          break;
        }
        case "done":
          setProgressLine("");
          break;
      }
    });

    const tick = window.setInterval(() => {
      const end = countdownEnd.current;
      if (end == null) return;
      const remain = Math.max(0, Math.ceil((end - Date.now()) / 1000));
      setProgressLine((prev) => {
        const m = prev.match(/^(.*等待 )\d+( 秒.*)$/);
        return m ? `${m[1]}${remain}${m[2]}` : prev;
      });
    }, 1000);
    return () => {
      unlistenPromise.then((fn) => fn()).catch(() => {});
      unlistenArticlePromise.then((fn) => fn()).catch(() => {});
      window.clearInterval(tick);
    };
  }, []);

  // modeRef 用于事件回调中读取最新的 active mode（事件回调闭包不会随 mode 重新注册）
  const modeRef = useRef<TaskMode>(mode);
  useEffect(() => {
    modeRef.current = mode;
  }, [mode]);

  // 按当前任务模式过滤目标节点。
  const targets = useMemo(
    () => leaves.filter((l) => l.leaf_type === TASK_META[mode].leafType),
    [leaves, mode]
  );

  const picked = pickedByMode[mode];
  const results = resultsByMode[mode];

  /** 是否已"完成"该 leaf。评论类看 commentedByMode，图文看 articleFinished。 */
  const isCompleted = (leafId: number): boolean => {
    if (mode === "article") return !!articleFinished[leafId];
    return !!commentedByMode[mode][leafId];
  };

  const loadLeaves = async (c: Course) => {
    setSelected(c);
    setPickedByMode({
      video_comment: new Set(),
      scored_discussion: new Set(),
      article: new Set(),
    });
    setResultsByMode({
      video_comment: [],
      scored_discussion: [],
      article: [],
    });
    setCommentedByMode({
      video_comment: {},
      scored_discussion: {},
      article: {},
    });
    setTopicTitleByMode({
      video_comment: {},
      scored_discussion: {},
      article: {},
    });
    setArticleFinished({});
    setLoading(true);
    try {
      const ls = await api.listChapters(c.classroom_id, c.sign);
      setLeaves(ls);
      // 并发拉取：
      // - 评论类节点（leaf_type=0 / leaf_type=4）→ batchMyCommentStatus
      // - 图文节点（leaf_type=3）→ courseSchedule 中 rate>=1 判定为已完成
      const videoLeafIds = ls
        .filter((l) => l.leaf_type === 0)
        .map((l) => l.id);
      const scoredDiscussionLeafIds = ls
        .filter((l) => l.leaf_type === 4)
        .map((l) => l.id);
      const articleLeafIds = ls
        .filter((l) => l.leaf_type === 3)
        .map((l) => l.id);

      if (
        videoLeafIds.length === 0 &&
        scoredDiscussionLeafIds.length === 0 &&
        articleLeafIds.length === 0
      ) {
        return;
      }

      setProbing(true);
      const tasks: Promise<void>[] = [];

      // splitTopicInfo: 把后端返回的 `{ leaf_id: { commented, title } }` 拆成
      // 两个并行的 `Record<number, ...>`，分别落入对应 mode 的 state。
      const splitTopicInfo = (res: Record<string, { commented: boolean; title: string }>) => {
        const commented: Record<number, boolean> = {};
        const titles: Record<number, string> = {};
        for (const [k, v] of Object.entries(res)) {
          const lid = Number(k);
          if (v.commented) commented[lid] = true;
          if (v.title) titles[lid] = v.title;
        }
        return { commented, titles };
      };

      if (videoLeafIds.length > 0) {
        tasks.push(
          api
            .batchMyCommentStatus(c.classroom_id, c.sign, videoLeafIds, 0)
            .then((res) => {
              const { commented, titles } = splitTopicInfo(res);
              setCommentedByMode((prev) => ({ ...prev, video_comment: commented }));
              setTopicTitleByMode((prev) => ({ ...prev, video_comment: titles }));
            })
            .catch((e) => {
              console.warn("batchMyCommentStatus(video) 失败", e);
            })
        );
      }
      if (scoredDiscussionLeafIds.length > 0) {
        tasks.push(
          api
            .batchMyCommentStatus(
              c.classroom_id,
              c.sign,
              scoredDiscussionLeafIds,
              4
            )
            .then((res) => {
              const { commented, titles } = splitTopicInfo(res);
              setCommentedByMode((prev) => ({
                ...prev,
                scored_discussion: commented,
              }));
              setTopicTitleByMode((prev) => ({
                ...prev,
                scored_discussion: titles,
              }));
            })
            .catch((e) => {
              console.warn("batchMyCommentStatus(discussion) 失败", e);
            })
        );
      }
      if (articleLeafIds.length > 0) {
        // 学堂在线没有专门的"图文完成状态批量查询"接口，但 course/schedule
        // 返回的 leaf_schedules 中 rate>=1 即为已学完，刚好覆盖图文。
        tasks.push(
          api
            .courseSchedule(c.classroom_id, c.sign)
            .then((sched) => {
              const next: Record<number, boolean> = {};
              for (const id of articleLeafIds) {
                const rate = sched[String(id)];
                if (typeof rate === "number" && rate >= 1) {
                  next[id] = true;
                }
              }
              setArticleFinished(next);
            })
            .catch((e) => {
              console.warn("courseSchedule(article) 失败", e);
            })
        );
      }

      Promise.allSettled(tasks).finally(() => setProbing(false));
    } finally {
      setLoading(false);
    }
  };

  const toggle = (id: number) => {
    setPickedByMode((prev) => {
      const n = new Set(prev[mode]);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return { ...prev, [mode]: n };
    });
  };

  const selectAllUnposted = () => {
    setPickedByMode((prev) => ({
      ...prev,
      [mode]: new Set(
        targets.filter((l) => !isCompleted(l.id)).map((l) => l.id)
      ),
    }));
  };

  const selectAll = () => {
    setPickedByMode((prev) => ({
      ...prev,
      [mode]: new Set(targets.map((l) => l.id)),
    }));
  };

  const clearSelection = () => {
    setPickedByMode((prev) => ({ ...prev, [mode]: new Set() }));
  };

  const send = async () => {
    if (!selected || picked.size === 0) return;
    const meta = TASK_META[mode];
    if (meta.requiresText && !text) {
      toast.info("请先填写要发送的评论文本");
      return;
    }
    setRunning(true);
    setResultsByMode((prev) => ({ ...prev, [mode]: [] }));
    setProgressLine(mode === "article" ? "准备标记…" : "准备发送…");
    try {
      let out: any[] = [];
      if (mode === "article") {
        out = await api.autoArticleLeaf(
          selected.classroom_id,
          selected.sku_id,
          selected.sign,
          Array.from(picked)
        );
      } else {
        out = await api.autoCommentLeaf(
          selected.classroom_id,
          selected.sign,
          Array.from(picked),
          text,
          {
            topic_type: meta.topicType,
            report_schedule: meta.reportSchedule,
            sku_id: meta.reportSchedule ? selected.sku_id : undefined,
          }
        );
      }
      setResultsByMode((prev) => ({ ...prev, [mode]: out }));
      const succeeded = new Set<number>(
        out.filter((r: any) => r?.ok).map((r: any) => Number(r.leaf_id))
      );
      if (succeeded.size > 0) {
        if (mode === "article") {
          setArticleFinished((m) => {
            const next = { ...m };
            succeeded.forEach((id) => {
              next[id] = true;
            });
            return next;
          });
        } else {
          setCommentedByMode((prev) => {
            const next = { ...prev[mode] };
            succeeded.forEach((id) => {
              next[id] = true;
            });
            return { ...prev, [mode]: next };
          });
        }
      }
      const failCount = out.length - succeeded.size;
      const verb = mode === "article" ? "标记完成" : "发送评论";
      if (failCount === 0) {
        toast.success(`已成功${verb} ${succeeded.size} 条`);
      } else if (succeeded.size === 0) {
        toast.error(`全部 ${failCount} 条${verb}失败`);
      } else {
        toast.info(
          `${verb}：成功 ${succeeded.size} 条，失败 ${failCount} 条`
        );
      }
    } catch (e: any) {
      setResultsByMode((prev) => ({
        ...prev,
        [mode]: [{ ok: false, error: String(e) }],
      }));
      toast.error(String(e));
    } finally {
      setRunning(false);
      setProgressLine("");
    }
  };

  const unpostedCount = useMemo(
    () => targets.filter((l) => !isCompleted(l.id)).length,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [targets, commentedByMode, articleFinished, mode]
  );
  const completedCount = targets.length - unpostedCount;

  const meta = TASK_META[mode];

  const sendBtnLabel = (() => {
    if (running) return mode === "article" ? "标记中…" : "发送中…";
    const action = mode === "article" ? "标记" : "发送";
    return `${action} ${picked.size} 个节点`;
  })();

  const completedLabel = mode === "article" ? "已完成" : "已评论";
  const unpostedLabel = mode === "article" ? "未完成" : "未评论";

  return (
    <div>
      <SectionTitle title="讨论与图文" subtitle={meta.subtitle} />

      {/* 任务类型 tab：切换 mode 即切换目标节点过滤、操作类型 */}
      <div className="px-12 mb-4 flex gap-3 flex-wrap">
        {TASK_ORDER.map((k) => {
          const m = TASK_META[k];
          return (
            <button
              key={k}
              type="button"
              onClick={() => setMode(k)}
              className={`text-caption px-4 py-2 rounded-pill border transition ${
                mode === k
                  ? "bg-action-blue text-white border-action-blue"
                  : "bg-white text-ink-muted-80 border-divider-soft hover:border-action-blue"
              }`}
            >
              {m.label}
            </button>
          );
        })}
      </div>

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
        <div className="px-12 grid grid-cols-1 lg:grid-cols-3 gap-6">
          <Card className="lg:col-span-2">
            <div className="flex items-center justify-between mb-3 flex-wrap gap-2">
              <div className="font-display text-tagline">
                选择节点
                <span className="ml-3 text-caption text-ink-muted-48 font-text">
                  共 {targets.length} 个 · {unpostedLabel} {unpostedCount} · {completedLabel}{" "}
                  {completedCount}
                  {probing && <span className="ml-2 anim-pulse">检测中…</span>}
                </span>
              </div>
              {loading && <Spinner />}
            </div>
            <div className="max-h-[420px] overflow-auto divide-y divide-divider-soft">
              {targets.map((l) => {
                const completed = isCompleted(l.id);
                // 评论类节点（视频讨论 / 带分加讨论）优先展示从 forum/unit/discussion
                // 提取的 content 预览，让"案例分析"这类同名节点可读地区分开。
                // 图文节点 fetched title 永远为空，自然走 leaf.name。
                const fetchedTitle =
                  mode === "article" ? "" : topicTitleByMode[mode][l.id] || "";
                const displayTitle = fetchedTitle || l.name;
                return (
                  <label
                    key={l.id}
                    className="flex items-center gap-3 py-2 cursor-pointer hover:bg-parchment/60 -mx-2 px-2 rounded-sm"
                  >
                    <input
                      type="checkbox"
                      checked={picked.has(l.id)}
                      onChange={() => toggle(l.id)}
                    />
                    <div className="flex-1 min-w-0">
                      <div className="text-body truncate flex items-center gap-2">
                        <span className="truncate" title={displayTitle}>
                          {displayTitle}
                        </span>
                        {completed && (
                          <span className="inline-flex items-center text-fine leading-none h-[20px] px-2 text-action-blue bg-action-blue/10 rounded-pill whitespace-nowrap">
                            {completedLabel}
                          </span>
                        )}
                      </div>
                      <div className="text-fine text-ink-muted-48 truncate">
                        {/* 章节路径 + 节点原始 name + 类型，让 fetched title 与 章节定位互相印证 */}
                        {l.chapter_path.join(" / ")}
                        {fetchedTitle ? ` / ${l.name}` : ""} · 类型 {l.leaf_type}
                      </div>
                    </div>
                  </label>
                );
              })}
              {!loading && targets.length === 0 && (
                <div className="text-body text-ink-muted-80 py-6">
                  本课程没有可作为「{meta.label}」目标的节点。
                </div>
              )}
            </div>
            <div className="flex items-center gap-3 mt-4 flex-wrap">
              <button
                className="text-link text-caption disabled:text-ink-muted-48 disabled:no-underline"
                onClick={selectAllUnposted}
                disabled={running || targets.length === 0}
                type="button"
              >
                全选{unpostedLabel}
              </button>
              <button
                className="text-link text-caption disabled:text-ink-muted-48 disabled:no-underline"
                onClick={selectAll}
                disabled={running || targets.length === 0}
                type="button"
              >
                全选所有
              </button>
              <button
                className="text-link text-caption disabled:text-ink-muted-48 disabled:no-underline"
                onClick={clearSelection}
                disabled={running || picked.size === 0}
                type="button"
              >
                全不选
              </button>
            </div>
          </Card>
          <Card>
            <div className="font-display text-tagline mb-3">
              {meta.requiresText ? "评论内容" : "操作"}
            </div>
            {meta.requiresText ? (
              <textarea
                className="field min-h-[160px]"
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder="输入要批量发送的评论文本"
              />
            ) : (
              <div className="text-caption text-ink-muted-80 leading-relaxed">
                图文节点不需要文本输入。点击下方按钮后，将依次向所选节点上报
                <code className="px-1 mx-1 bg-parchment rounded">
                  chapter/schedule
                </code>
                ，让学堂在线把这些节点的学习进度记为已完成。
              </div>
            )}
            <Pill
              className="mt-4"
              onClick={send}
              disabled={
                running || picked.size === 0 || (meta.requiresText && !text)
              }
            >
              {sendBtnLabel}
            </Pill>
            {running && progressLine && (
              <div className="mt-3 text-caption text-ink-muted-80 anim-pulse">
                {progressLine}
              </div>
            )}
            <div className="mt-4 space-y-2 max-h-[260px] overflow-auto">
              {results.map((r, i) => (
                <div
                  key={i}
                  className={`text-caption ${r.ok ? "text-action-blue" : "text-[#cc2b2b]"}`}
                >
                  leaf {r.leaf_id} · {r.ok ? "成功" : `失败：${r.error}`}
                  {r.ok && r.schedule_error && (
                    <span className="ml-2 text-fine text-ink-muted-48">
                      （schedule 上报失败：{r.schedule_error}）
                    </span>
                  )}
                </div>
              ))}
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}
