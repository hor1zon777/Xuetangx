import { useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  onBankUpdated,
  type BankEntry,
  type BankStats,
  type ProblemKind,
} from "../lib/api";
import { Card, Pill, SectionTitle, Spinner } from "../components/ui";
import { KindBadge, kindLabel } from "../components/KindBadge";
import { RefreshIcon } from "../components/icons";
import { toast } from "../components/Toast";

const sourceLabel: Record<string, string> = {
  xuetang: "学堂确认",
  manual: "手动导入",
};

function formatTime(unix: number): string {
  if (!unix) return "—";
  const d = new Date(unix * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * 把 BankEntry 的答案格式化成可读字符串：
 * - 选项题：直接拼 keys（["A","C"] → "AC"；["true"] → "true"）
 * - 文本题：原样展示
 */
function formatAnswer(e: BankEntry): string {
  if (e.answer && e.answer.length > 0) return e.answer.join("");
  if (e.answer_text) return e.answer_text;
  return "—";
}

export function BankPage() {
  const [stats, setStats] = useState<BankStats | null>(null);
  const [list, setList] = useState<BankEntry[]>([]);
  const [keyword, setKeyword] = useState("");
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<BankEntry | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  // 用 ref 持有最新关键词，让事件订阅回调始终拿到当前值；
  // 否则订阅 effect 依赖 [keyword] 会反复 unlisten/listen，频繁 IPC。
  const keywordRef = useRef(keyword);
  keywordRef.current = keyword;

  /**
   * 拉取最新统计 + 题库列表。
   * @param opts.silent 静默模式：不显示 loading spinner。事件触发的自动刷新走静默，
   *                    避免页面频繁闪烁；用户主动点击刷新则显示 spinner 提示「正在刷新」。
   */
  const refresh = async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) setLoading(true);
    try {
      const [s, l] = await Promise.all([
        api.bankStats(),
        api.bankList(keywordRef.current || undefined, 0, 500),
      ]);
      setStats(s);
      setList(l);
    } catch (e: any) {
      toast.error(`加载题库失败：${e}`);
    } finally {
      if (!opts?.silent) setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 关键词输入做轻量去抖（300ms）后重新查询
  useEffect(() => {
    const t = setTimeout(() => {
      refresh();
    }, 300);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keyword]);

  // 订阅"题库内容变更"事件：手动收录 / 自动作业入库 / 删除 / 清空 / 导入
  // 都会派发此事件，订阅后页面自动刷新，无需用户切回来再手动点。
  useEffect(() => {
    const unlistenPromise = onBankUpdated(() => {
      refresh({ silent: true });
    });
    return () => {
      unlistenPromise.then((fn) => fn()).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onDelete = async (e: BankEntry) => {
    if (!confirm(`确认从题库中删除 problem_id=${e.problem_id} 的题目？`)) return;
    try {
      const ok = await api.bankDelete(e.problem_id);
      if (ok) {
        toast.success("已删除");
        if (selected?.problem_id === e.problem_id) setSelected(null);
        // 不再显式 refresh：后端会派发 bank://updated 由事件统一刷新
      } else {
        toast.info("题目不存在");
      }
    } catch (err: any) {
      toast.error(`删除失败：${err}`);
    }
  };

  const onClear = async () => {
    if (!stats || stats.total === 0) {
      toast.info("题库已经是空的");
      return;
    }
    if (!confirm(`确认清空全部 ${stats.total} 道题目？此操作不可恢复。`)) return;
    try {
      const n = await api.bankClear();
      toast.success(`已清空 ${n} 道题`);
      setSelected(null);
    } catch (e: any) {
      toast.error(`清空失败：${e}`);
    }
  };

  const onExport = async () => {
    try {
      const entries = await api.bankExport();
      const blob = new Blob([JSON.stringify(entries, null, 2)], {
        type: "application/json",
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      const ts = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
      a.href = url;
      a.download = `xuetang-bank-${ts}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      toast.success(`已导出 ${entries.length} 道题`);
    } catch (e: any) {
      toast.error(`导出失败：${e}`);
    }
  };

  const onImport = async (file: File) => {
    try {
      const text = await file.text();
      const parsed = JSON.parse(text);
      if (!Array.isArray(parsed)) {
        toast.error("文件格式不正确：需要 BankEntry 数组");
        return;
      }
      const r = await api.bankImport(parsed as BankEntry[]);
      toast.success(
        `导入完成：新增 ${r.added} · 更新 ${r.updated} · 跳过 ${r.skipped}（总计 ${r.total_after}）`
      );
      // 不再显式 refresh：bank_import 后端会 emit bank://updated 触发自动刷新
    } catch (e: any) {
      toast.error(`导入失败：${e}`);
    }
  };

  const orderedKinds = useMemo(() => {
    if (!stats) return [] as [string, number][];
    return Object.entries(stats.by_kind).sort((a, b) => b[1] - a[1]);
  }, [stats]);

  return (
    <div>
      <SectionTitle
        title="本地题库"
        subtitle="存储所有已批改作业里学堂返回的标准答案。自动作业时优先使用本地答案，命中后跳过 AI 询问。"
      />

      <div className="px-12 grid grid-cols-1 lg:grid-cols-3 gap-6 pb-8">
        <Card>
          <div className="font-display text-tagline mb-3">概览</div>
          {!stats ? (
            <div className="text-body text-ink-muted-80">加载中…</div>
          ) : (
            <div className="space-y-3">
              <div className="text-display-md font-display text-ink leading-none">
                {stats.total}
                <span className="ml-2 text-tagline text-ink-muted-48">道</span>
              </div>
              <div className="text-caption text-ink-muted-80">
                累计命中 <span className="text-ink">{stats.total_hits}</span> 次
              </div>
              {orderedKinds.length > 0 && (
                <div>
                  <div className="text-fine text-ink-muted-48 mb-1.5">题型分布</div>
                  <div className="flex flex-wrap gap-1.5">
                    {orderedKinds.map(([k, n]) => (
                      <KindBadge key={k} kind={k as ProblemKind} text={`${kindLabel(k as ProblemKind)} ${n}`} />
                    ))}
                  </div>
                </div>
              )}
              {Object.keys(stats.by_source).length > 0 && (
                <div>
                  <div className="text-fine text-ink-muted-48 mb-1.5">来源</div>
                  <div className="flex flex-wrap gap-1.5">
                    {Object.entries(stats.by_source).map(([k, n]) => (
                      <span
                        key={k}
                        className="inline-flex items-center text-fine leading-none h-[20px] px-2 bg-parchment text-ink-muted-80 rounded-pill whitespace-nowrap"
                      >
                        {sourceLabel[k] ?? k} {n}
                      </span>
                    ))}
                  </div>
                </div>
              )}
              <div className="pt-2 flex flex-wrap gap-2">
                <Pill variant="ghost" onClick={onExport}>
                  导出 JSON
                </Pill>
                <Pill
                  variant="ghost"
                  onClick={() => fileRef.current?.click()}
                >
                  导入 JSON
                </Pill>
                <input
                  ref={fileRef}
                  type="file"
                  accept="application/json,.json"
                  className="hidden"
                  onChange={(e) => {
                    const f = e.target.files?.[0];
                    if (f) {
                      onImport(f);
                      e.target.value = "";
                    }
                  }}
                />
                <Pill variant="ghost" onClick={onClear}>
                  清空
                </Pill>
              </div>
            </div>
          )}
        </Card>

        <Card className="lg:col-span-2">
          <div className="flex items-center justify-between gap-3 mb-3 flex-wrap">
            <div className="font-display text-tagline">
              题目列表
              {list.length > 0 && (
                <span className="ml-2 text-caption text-ink-muted-48 font-text">
                  显示 {list.length} 道
                </span>
              )}
            </div>
            <div className="flex items-center gap-2 flex-wrap">
              <input
                className="field"
                placeholder="搜索题面 / problem_id / 答案"
                value={keyword}
                onChange={(e) => setKeyword(e.target.value)}
                style={{ width: 260 }}
              />
              <button
                type="button"
                className="text-link text-caption inline-flex items-center gap-1 disabled:text-ink-muted-48 disabled:no-underline"
                onClick={() => refresh()}
                disabled={loading}
                title="重新拉取最新题库"
              >
                <RefreshIcon className="w-3.5 h-3.5" />
                刷新
              </button>
              {loading && <Spinner />}
            </div>
          </div>
          <div className="max-h-[520px] overflow-auto divide-y divide-divider-soft">
            {list.map((e) => (
              <button
                key={e.problem_id}
                type="button"
                onClick={() => setSelected(e)}
                className={`block w-full text-left py-2 -mx-2 px-2 rounded-sm transition-colors ${selected?.problem_id === e.problem_id ? "bg-action-blue/5" : "hover:bg-parchment/60"}`}
              >
                <div className="flex items-start gap-2">
                  <KindBadge kind={e.kind} />
                  <div className="flex-1 min-w-0">
                    <div className="text-body truncate">{e.body_preview || "（无题面预览）"}</div>
                    <div className="text-fine text-ink-muted-48 flex flex-wrap gap-x-3 gap-y-0.5 mt-0.5">
                      <span>ID {e.problem_id}</span>
                      <span>答 {formatAnswer(e)}</span>
                      <span>{sourceLabel[e.source] ?? e.source}</span>
                      <span>命中 {e.hit_count}</span>
                      <span>{formatTime(e.updated_at)}</span>
                    </div>
                  </div>
                </div>
              </button>
            ))}
            {!loading && list.length === 0 && (
              <div className="text-body text-ink-muted-80 py-8 text-center">
                {keyword ? "没有匹配的题目" : "题库还是空的，去自动作业页『收录已完成节点』填充一下吧"}
              </div>
            )}
          </div>
        </Card>
      </div>

      {selected && (
        <div className="px-12 pb-8">
          <Card>
            <div className="flex items-center justify-between gap-3 mb-3 flex-wrap">
              <div className="font-display text-tagline">
                题目详情
                <span className="ml-2 text-caption text-ink-muted-48 font-text">
                  problem_id = {selected.problem_id}
                </span>
              </div>
              <div className="flex items-center gap-3">
                <button className="text-link text-caption" onClick={() => setSelected(null)}>
                  关闭
                </button>
                <button
                  className="text-link text-caption text-[#cc2b2b]"
                  onClick={() => onDelete(selected)}
                >
                  删除此题
                </button>
              </div>
            </div>
            <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-caption">
              <dt className="text-ink-muted-80">题型</dt>
              <dd>
                <KindBadge kind={selected.kind} />
              </dd>
              <dt className="text-ink-muted-80">题面</dt>
              <dd className="text-ink whitespace-pre-wrap break-words">{selected.body_preview}</dd>
              {selected.option_keys.length > 0 && (
                <>
                  <dt className="text-ink-muted-80">选项 keys</dt>
                  <dd className="text-ink font-mono">{selected.option_keys.join(" / ")}</dd>
                </>
              )}
              <dt className="text-ink-muted-80">答案</dt>
              <dd className="text-ink font-mono break-all">{formatAnswer(selected)}</dd>
              <dt className="text-ink-muted-80">body_hash</dt>
              <dd className="text-ink-muted-48 font-mono text-fine break-all">{selected.body_hash}</dd>
              <dt className="text-ink-muted-80">来源</dt>
              <dd className="text-ink">{sourceLabel[selected.source] ?? selected.source}</dd>
              <dt className="text-ink-muted-80">命中次数</dt>
              <dd className="text-ink">{selected.hit_count}</dd>
              <dt className="text-ink-muted-80">更新时间</dt>
              <dd className="text-ink">{formatTime(selected.updated_at)}</dd>
            </dl>
          </Card>
        </div>
      )}
    </div>
  );
}
