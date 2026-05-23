import { useEffect, useMemo, useRef, useState } from "react";
import { api, type BankEntry, type BankStats, type ProblemKind } from "../lib/api";
import { Card, Pill, SectionTitle, Spinner } from "../components/ui";
import { KindBadge, kindLabel } from "../components/KindBadge";
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

  const refresh = async () => {
    setLoading(true);
    try {
      const [s, l] = await Promise.all([api.bankStats(), api.bankList(keyword || undefined, 0, 500)]);
      setStats(s);
      setList(l);
    } catch (e: any) {
      toast.error(`加载题库失败：${e}`);
    } finally {
      setLoading(false);
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

  const onDelete = async (e: BankEntry) => {
    if (!confirm(`确认从题库中删除 problem_id=${e.problem_id} 的条目？`)) return;
    try {
      const ok = await api.bankDelete(e.problem_id);
      if (ok) {
        toast.success("已删除");
        if (selected?.problem_id === e.problem_id) setSelected(null);
        refresh();
      } else {
        toast.info("条目不存在");
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
    if (!confirm(`确认清空全部 ${stats.total} 条题库记录？此操作不可恢复。`)) return;
    try {
      const n = await api.bankClear();
      toast.success(`已清空 ${n} 条`);
      setSelected(null);
      refresh();
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
      toast.success(`已导出 ${entries.length} 条`);
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
      refresh();
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
                <span className="ml-2 text-tagline text-ink-muted-48">条</span>
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
              条目列表
              {list.length > 0 && (
                <span className="ml-2 text-caption text-ink-muted-48 font-text">
                  显示 {list.length} 条
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
                {keyword ? "没有匹配的条目" : "题库还是空的，去自动作业页『收录已完成节点』填充一下吧"}
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
                条目详情
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
                  删除此条
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
