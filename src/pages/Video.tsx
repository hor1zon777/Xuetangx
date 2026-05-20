import { useEffect, useMemo } from "react";
import { api, type Course } from "../lib/api";
import type { PendingTask, VideoActions, VideoState } from "../lib/videoState";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";
import { toast } from "../components/Toast";

export function VideoPage({ state }: { state: VideoState & VideoActions }) {
  const {
    courses,
    selected,
    leaves,
    schedule,
    tasks,
    picked,
    speed,
    hideFinished,
    loading,
    submitting,
    error,
    setCourses,
    setSelected,
    setLeaves,
    setSchedule,
    setPicked,
    setSpeed,
    setHideFinished,
    setLoading,
    setSubmitting,
    setError,
    setTasks,
  } = state;

  // 用 id → name 查表，便于 pending 卡片即时显示
  const leafNameMap = useMemo(() => {
    const m: Record<number, string> = {};
    for (const l of leaves) m[l.id] = l.name;
    return m;
  }, [leaves]);

  const allVideos = useMemo(() => leaves.filter((l) => l.leaf_type === 0), [leaves]);
  const videoLeaves = useMemo(
    () =>
      hideFinished
        ? allVideos.filter((l) => (schedule[String(l.id)] ?? 0) < 1)
        : allVideos,
    [allVideos, schedule, hideFinished]
  );
  const finishedCount = useMemo(
    () => allVideos.filter((l) => (schedule[String(l.id)] ?? 0) >= 1).length,
    [allVideos, schedule]
  );

  // 首次进入：拉课程列表（若状态里还没有）
  useEffect(() => {
    if (courses.length === 0) {
      api.listCourses().then(setCourses).catch((e) => setError(String(e)));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadLeaves = async (c: Course) => {
    setSelected(c);
    setLoading(true);
    setPicked(new Set());
    setError(null);
    try {
      const [ls, sched] = await Promise.all([
        api.listChapters(c.classroom_id, c.sign),
        api.courseSchedule(c.classroom_id, c.sign).catch(() => ({})),
      ]);
      setLeaves(ls);
      setSchedule(sched);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const isFinished = (id: number) => (schedule[String(id)] ?? 0) >= 1;

  const toggle = (id: number) => {
    if (isFinished(id)) return;
    const n = new Set(picked);
    if (n.has(id)) n.delete(id);
    else n.add(id);
    setPicked(n);
  };

  const selectAllUnfinished = () => {
    setPicked(new Set(videoLeaves.filter((l) => !isFinished(l.id)).map((l) => l.id)));
  };

  const clearSelection = () => setPicked(new Set());

  const refreshSchedule = async () => {
    if (!selected) return;
    try {
      setSchedule(await api.courseSchedule(selected.classroom_id, selected.sign));
      toast.info("进度已刷新");
    } catch (e: any) {
      setError(String(e));
    }
  };

  const startAll = async () => {
    if (!selected || picked.size === 0) return;
    setSubmitting(true);

    const targets = Array.from(picked).filter((id) => !isFinished(id));
    const pendings: PendingTask[] = targets.map((id) => ({
      task_id: `pending-${id}-${Date.now()}`,
      pending: true,
      leaf_id: id,
      leaf_name: leafNameMap[id] ?? null,
      classroom_id: selected.classroom_id,
      current_pos: 0,
      duration: 0,
      finished: false,
      error: null,
    }));
    setTasks((arr) => [...arr, ...pendings]);
    setPicked(new Set());
    toast.info(`已提交 ${targets.length} 个任务，正在解析视频…`);

    let ok = 0;
    const errs: string[] = [];
    await Promise.all(
      targets.map(async (id) => {
        try {
          await api.startVideoTask({
            classroom_id: selected.classroom_id,
            sku_id: selected.sku_id,
            sign: selected.sign,
            leaf_id: id,
            speed,
            leaf_name: leafNameMap[id],
          });
          ok++;
        } catch (err: any) {
          const msg = String(err);
          errs.push(`${leafNameMap[id] ?? `leaf ${id}`}：${msg}`);
          setTasks((arr) =>
            arr.map((t) =>
              t.pending && t.leaf_id === id
                ? { ...t, finished: true, error: msg }
                : t
            )
          );
        }
      })
    );
    setSubmitting(false);
    if (errs.length) {
      toast.error(`${errs.length} 个任务启动失败`);
      setError(errs.join("\n"));
    } else if (ok > 0) {
      toast.success(`${ok} 个任务已开始`);
    }
  };

  const stop = async (id: string) => {
    await api.stopVideoTask(id);
    toast.info("已停止");
  };

  const removeFinished = () => {
    setTasks((arr) => arr.filter((t) => !t.finished));
  };

  return (
    <div>
      <SectionTitle
        title="自动观看视频"
        subtitle="选择课程 → 勾选未完成视频 → 一键开始模拟心跳。已完成视频会自动跳过。"
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
                视频节点
                <span className="ml-3 text-caption text-ink-muted-48 font-text">
                  共 {allVideos.length} 个，已完成 {finishedCount}
                </span>
              </div>
              <div className="flex items-center gap-3">
                {loading && <Spinner />}
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
            {error && (
              <div className="text-caption text-[#cc2b2b] mb-2 whitespace-pre-line">
                {error}
              </div>
            )}
            <div className="max-h-[420px] overflow-auto divide-y divide-divider-soft">
              {videoLeaves.map((l) => {
                const rate = schedule[String(l.id)] ?? 0;
                const finished = rate >= 1;
                return (
                  <label
                    key={l.id}
                    className={`flex items-center gap-3 py-2 transition-colors ${
                      finished
                        ? "opacity-50 cursor-not-allowed"
                        : "cursor-pointer hover:bg-parchment/60 -mx-2 px-2 rounded-sm"
                    }`}
                  >
                    <input
                      type="checkbox"
                      checked={picked.has(l.id)}
                      disabled={finished}
                      onChange={() => toggle(l.id)}
                    />
                    <div className="flex-1 min-w-0">
                      <div className="text-body text-ink truncate flex items-center gap-2">
                        {l.name}
                        {finished && (
                          <span className="inline-flex items-center text-fine text-action-blue bg-action-blue/10 px-2 py-0.5 rounded-pill">
                            已完成
                          </span>
                        )}
                        {!finished && rate > 0 && (
                          <span className="inline-flex items-center text-fine text-ink-muted-80 bg-parchment px-2 py-0.5 rounded-pill">
                            {Math.round(rate * 100)}%
                          </span>
                        )}
                      </div>
                      <div className="text-fine text-ink-muted-48 truncate">
                        {l.chapter_path.join(" / ")}
                      </div>
                    </div>
                    <span className="text-fine text-ink-muted-48">id：{l.id}</span>
                  </label>
                );
              })}
              {!loading && videoLeaves.length === 0 && (
                <div className="text-body text-ink-muted-80 py-6">
                  {hideFinished && allVideos.length > 0
                    ? "所有视频均已完成。"
                    : "本课程无视频节点。"}
                </div>
              )}
            </div>
            <div className="flex items-center gap-3 mt-4 flex-wrap">
              <button
                className="text-link text-caption"
                onClick={selectAllUnfinished}
                type="button"
              >
                全选未完成
              </button>
              <button
                className="text-link text-caption"
                onClick={clearSelection}
                type="button"
              >
                清空
              </button>
              <span className="text-caption text-ink-muted-80">倍速</span>
              <input
                className="field max-w-[80px]"
                type="number"
                step={0.1}
                min={0.5}
                max={2}
                value={speed}
                onChange={(e) => setSpeed(Number(e.target.value))}
              />
              <Pill onClick={startAll} disabled={picked.size === 0 || submitting}>
                {submitting ? (
                  <span className="inline-flex items-center gap-2">
                    <Spinner /> 提交中…
                  </span>
                ) : (
                  `开始 ${picked.size} 个任务`
                )}
              </Pill>
            </div>
          </Card>
          <Card>
            <div className="flex items-center justify-between mb-3">
              <div className="font-display text-tagline">
                运行中任务
                <span className="ml-2 text-caption text-ink-muted-48 font-text">
                  {tasks.length}
                </span>
              </div>
              {tasks.some((t) => t.finished) && (
                <button
                  className="text-link text-caption"
                  onClick={removeFinished}
                >
                  清空已完成
                </button>
              )}
            </div>
            <div className="space-y-3 max-h-[480px] overflow-auto">
              {tasks.map((t) => {
                const pct =
                  t.pending
                    ? 0
                    : t.duration > 0
                    ? Math.min(100, (t.current_pos / t.duration) * 100)
                    : t.finished && !t.error
                    ? 100
                    : 0;
                return (
                  <div
                    key={t.task_id}
                    className="border border-hairline rounded-md p-3 anim-in"
                  >
                    <div className="text-body text-ink truncate flex items-center gap-2">
                      <span className="truncate flex-1">
                        {t.leaf_name || `leaf ${t.leaf_id}`}
                      </span>
                      {t.pending && (
                        <span className="text-fine text-action-blue anim-pulse">
                          准备中
                        </span>
                      )}
                    </div>
                    <div className="text-fine text-ink-muted-48 flex justify-between mt-1">
                      <span>id：{t.leaf_id}</span>
                      <span>
                        {t.pending
                          ? "正在解析视频…"
                          : `${Math.round(t.current_pos)}s / ${Math.round(
                              t.duration
                            )}s`}
                      </span>
                    </div>
                    <div className="relative w-full h-1.5 bg-divider-soft rounded-pill mt-2 overflow-hidden">
                      <div
                        className="absolute inset-y-0 left-0 bg-action-blue progress-bar-fill"
                        style={{ width: `${pct}%` }}
                      />
                      {t.pending && (
                        <div className="absolute inset-0 shimmer rounded-pill" />
                      )}
                    </div>
                    <div className="flex justify-between items-center mt-2 text-fine">
                      <span
                        className={
                          t.error
                            ? "text-[#cc2b2b]"
                            : t.finished
                            ? "text-action-blue"
                            : t.pending
                            ? "text-ink-muted-48"
                            : "text-ink-muted-80"
                        }
                      >
                        {t.error
                          ? `失败：${t.error}`
                          : t.finished
                          ? "已完成"
                          : t.pending
                          ? "等待心跳建立…"
                          : "进行中"}
                      </span>
                      {!t.finished && !t.pending && (
                        <button
                          className="text-link"
                          onClick={() => stop(t.task_id)}
                        >
                          停止
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
              {tasks.length === 0 && (
                <div className="text-body text-ink-muted-80">暂无任务。</div>
              )}
            </div>
          </Card>
        </div>
      )}
    </div>
  );
}
