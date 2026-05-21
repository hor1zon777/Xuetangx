import { useEffect, useMemo, useRef, useState } from "react";
import { api, onForumProgress, type Course, type ForumProgress, type LeafNode } from "../lib/api";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";
import { toast } from "../components/Toast";

export function ForumPage() {
  const [courses, setCourses] = useState<Course[]>([]);
  const [selected, setSelected] = useState<Course | null>(null);
  const [leaves, setLeaves] = useState<LeafNode[]>([]);
  const [loading, setLoading] = useState(false);
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [text, setText] = useState("");
  const [results, setResults] = useState<any[]>([]);
  const [running, setRunning] = useState(false);
  // 限速进度提示：根据 forum://progress 事件实时显示"等待 N 秒"或"重试中"
  const [progressLine, setProgressLine] = useState<string>("");
  // 倒计时用：把 rate_limited 的 retry_after 渲染成"剩余 N 秒"动态计数
  const countdownEnd = useRef<number | null>(null);
  // 已评论检测状态：
  // - commentedMap: { [leaf_id]: true } 表示当前账号已在该节点评论过
  //   未出现的 leaf 视为"未评论"或"检测失败"，统一按未评论处理（不阻塞用户操作）。
  // - probing: 是否正在批量检测，UI 上显示一个"检测中…"提示。
  const [commentedMap, setCommentedMap] = useState<Record<number, boolean>>({});
  const [probing, setProbing] = useState(false);

  useEffect(() => {
    api.listCourses().then(setCourses);
    api.getSettings().then((s) => setText(s.auto_comment_default || ""));
  }, []);

  // 订阅后端 forum://progress 进度事件，把当前阶段渲染成人话。
  // 由于后端在 sleep retry_after 期间不会持续 emit，UI 用 setInterval 自己跑倒计时显示。
  //
  // 重要：onForumProgress 是异步注册的，必须把 Promise 本身保留下来在 cleanup 里
  // `then(fn => fn())`，而不能依赖一个 `let unlisten` 局部变量。否则在 React 18
  // 严格模式（mount→unmount→mount）下，第一次 mount 的 cleanup 触发时 Promise
  // 还没 resolve，`unlisten` 仍是 undefined，于是第一个 listener 没被取消、
  // 第二次 mount 又注册了第二个 listener —— 每条 item 事件都会被消费两次，
  // 表现就是结果列表里同一条 leaf "成功" 重复出现。
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
          // 单条任务彻底结束（成功 / 失败），立即追加到结果列表。
          // 这样用户不必等整批 await 完才看到状态：每发完一条就刷一条出来。
          const result = p.extra as any;
          if (result && typeof result === "object" && "leaf_id" in result) {
            setResults((prev) => [...prev, result]);
            if (result.ok) {
              const lid = Number(result.leaf_id);
              setCommentedMap((m) => (m[lid] ? m : { ...m, [lid]: true }));
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
    // 1Hz 刷新限速倒计时，让"等待 N 秒"动态减小
    const tick = window.setInterval(() => {
      const end = countdownEnd.current;
      if (end == null) return;
      const remain = Math.max(0, Math.ceil((end - Date.now()) / 1000));
      setProgressLine((prev) => {
        // 仅替换数字部分，保留前缀；如果格式被覆盖了就不动
        const m = prev.match(/^(.*等待 )\d+( 秒.*)$/);
        return m ? `${m[1]}${remain}${m[2]}` : prev;
      });
    }, 1000);
    return () => {
      // 等 Promise resolve 后再调用 unlisten；即便 cleanup 跑在注册完成之前，
      // 注册完成后也会马上被取消，不会留下幽灵 listener。
      unlistenPromise.then((fn) => fn()).catch(() => {});
      window.clearInterval(tick);
    };
  }, []);

  // 仅展示视频节点（leaf_type=0）。
  // 学堂在线的"习题"(leaf_type=6) 走的是 exercise 体系，并没有 unit/discussion，
  // 之前一并显示会让用户误以为这类节点也能评论（截图里"项目一--院前急救--习题"
  // 还会被标注成"已评论"，实际只是后端在错的话题上撞了一条记录）。
  const targets = useMemo(
    () => leaves.filter((l) => l.leaf_type === 0),
    [leaves]
  );

  const loadLeaves = async (c: Course) => {
    setSelected(c);
    setPicked(new Set());
    setCommentedMap({});
    setResults([]);
    setLoading(true);
    try {
      const ls = await api.listChapters(c.classroom_id, c.sign);
      setLeaves(ls);
      // 异步并发检测每个有讨论区的节点是否已被当前账号评论过。
      // 失败不阻塞 UI，仅在 toast 提示一次（避免每个 leaf 单独失败刷屏）。
      // 注意：只对视频节点（leaf_type=0）发起检测，习题等其它类型没有讨论区。
      const probeIds = ls
        .filter((l) => l.leaf_type === 0)
        .map((l) => l.id);
      if (probeIds.length > 0) {
        setProbing(true);
        api
          .batchMyCommentStatus(c.classroom_id, c.sign, probeIds)
          .then((res) => {
            // 后端返回的 key 是字符串，转回 number 索引。
            const next: Record<number, boolean> = {};
            for (const [k, v] of Object.entries(res)) {
              if (v) next[Number(k)] = true;
            }
            setCommentedMap(next);
          })
          .catch((e) => {
            console.warn("批量检测已评论状态失败", e);
            toast.info("已评论状态检测失败，节点不会被自动标注");
          })
          .finally(() => setProbing(false));
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

  // 全选未评论：把所有"还没被当前账号评论过"的可选节点都勾上。
  // 已评论的节点维持当前的勾选状态（用户若想强制再发也可以手动勾），
  // 但默认不会被批量勾入，避免重复发评论。
  const selectAllUnposted = () => {
    setPicked(new Set(targets.filter((l) => !commentedMap[l.id]).map((l) => l.id)));
  };

  // 全选所有：包括已评论节点，给用户保留"我就是要再发一次"的能力。
  const selectAll = () => {
    setPicked(new Set(targets.map((l) => l.id)));
  };

  const clearSelection = () => setPicked(new Set());

  const send = async () => {
    if (!selected || picked.size === 0 || !text) return;
    setRunning(true);
    setResults([]);
    setProgressLine("准备发送…");
    try {
      // 不再在前端硬编码 delay_ms（旧值 1500ms 会立刻触发学堂的 60s/10 条限速）。
      // 把节流交给后端：默认 7s/条，命中 429 自适应回退到 12s 并按 retry-after 重试。
      const out = await api.autoCommentLeaf(
        selected.classroom_id,
        selected.sign,
        Array.from(picked),
        text
      );
      setResults(out);
      // 发完后把刚刚成功提交的节点标记为已评论，避免立刻重发。
      const succeeded = new Set<number>(
        out.filter((r: any) => r?.ok).map((r: any) => Number(r.leaf_id))
      );
      if (succeeded.size > 0) {
        setCommentedMap((m) => {
          const next = { ...m };
          succeeded.forEach((id) => {
            next[id] = true;
          });
          return next;
        });
      }
      // 顺手给个汇总 toast
      const failCount = out.length - succeeded.size;
      if (failCount === 0) {
        toast.success(`已成功发送 ${succeeded.size} 条评论`);
      } else if (succeeded.size === 0) {
        toast.error(`全部 ${failCount} 条发送失败`);
      } else {
        toast.info(`成功 ${succeeded.size} 条，失败 ${failCount} 条`);
      }
    } catch (e: any) {
      setResults([{ ok: false, error: String(e) }]);
      toast.error(String(e));
    } finally {
      setRunning(false);
      setProgressLine("");
    }
  };

  const unpostedCount = useMemo(
    () => targets.filter((l) => !commentedMap[l.id]).length,
    [targets, commentedMap]
  );
  const commentedCount = targets.length - unpostedCount;

  return (
    <div>
      <SectionTitle
        title="讨论区评论"
        subtitle="批量在选定节点的讨论区中发表同一条评论。已评论过的节点会自动标注，默认不在“全选未评论”中。"
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
        <div className="px-12 grid grid-cols-1 lg:grid-cols-3 gap-6">
          <Card className="lg:col-span-2">
            <div className="flex items-center justify-between mb-3 flex-wrap gap-2">
              <div className="font-display text-tagline">
                选择节点
                <span className="ml-3 text-caption text-ink-muted-48 font-text">
                  共 {targets.length} 个 · 未评论 {unpostedCount} · 已评论 {commentedCount}
                  {probing && <span className="ml-2 anim-pulse">检测中…</span>}
                </span>
              </div>
              {loading && <Spinner />}
            </div>
            <div className="max-h-[420px] overflow-auto divide-y divide-divider-soft">
              {targets.map((l) => {
                const commented = !!commentedMap[l.id];
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
                        <span className="truncate">{l.name}</span>
                        {commented && (
                          <span className="inline-flex items-center text-fine leading-none h-[20px] px-2 text-action-blue bg-action-blue/10 rounded-pill whitespace-nowrap">
                            已评论
                          </span>
                        )}
                      </div>
                      <div className="text-fine text-ink-muted-48">
                        {l.chapter_path.join(" / ")} · 类型 {l.leaf_type}
                      </div>
                    </div>
                  </label>
                );
              })}
              {!loading && targets.length === 0 && (
                <div className="text-body text-ink-muted-80 py-6">
                  本课程无可评论的节点。
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
                全选未评论
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
            <div className="font-display text-tagline mb-3">评论内容</div>
            <textarea
              className="field min-h-[160px]"
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder="输入要批量发送的评论文本"
            />
            <Pill
              className="mt-4"
              onClick={send}
              disabled={running || picked.size === 0 || !text}
            >
              {running ? "发送中…" : `在 ${picked.size} 个节点发送`}
            </Pill>
            {/* 限速 / 进度提示：仅在发送过程中显示，不与结果列表抢空间。 */}
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
                </div>
              ))}
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}
