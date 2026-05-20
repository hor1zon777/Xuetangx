import { useEffect, useMemo, useState } from "react";
import { api, type Course, type LeafNode } from "../lib/api";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";

export function ForumPage() {
  const [courses, setCourses] = useState<Course[]>([]);
  const [selected, setSelected] = useState<Course | null>(null);
  const [leaves, setLeaves] = useState<LeafNode[]>([]);
  const [loading, setLoading] = useState(false);
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [text, setText] = useState("");
  const [results, setResults] = useState<any[]>([]);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    api.listCourses().then(setCourses);
    api.getSettings().then((s) => setText(s.auto_comment_default || ""));
  }, []);

  const targets = useMemo(() => leaves.filter((l) => l.leaf_type === 0 || l.leaf_type === 6), [leaves]);

  const loadLeaves = async (c: Course) => {
    setSelected(c);
    setPicked(new Set());
    setLoading(true);
    try {
      setLeaves(await api.listChapters(c.classroom_id, c.sign));
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

  const send = async () => {
    if (!selected || picked.size === 0 || !text) return;
    setRunning(true);
    setResults([]);
    try {
      const out = await api.autoCommentLeaf(
        selected.classroom_id,
        selected.sign,
        Array.from(picked),
        text,
        1500
      );
      setResults(out);
    } catch (e: any) {
      setResults([{ ok: false, error: String(e) }]);
    } finally {
      setRunning(false);
    }
  };

  return (
    <div>
      <SectionTitle
        title="讨论区评论"
        subtitle="批量在选定节点的讨论区中发表同一条评论。"
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
            <div className="flex items-center justify-between mb-3">
              <div className="font-display text-tagline">选择节点</div>
              {loading && <Spinner />}
            </div>
            <div className="max-h-[420px] overflow-auto divide-y divide-divider-soft">
              {targets.map((l) => (
                <label key={l.id} className="flex items-center gap-3 py-2 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={picked.has(l.id)}
                    onChange={() => toggle(l.id)}
                  />
                  <div className="flex-1 min-w-0">
                    <div className="text-body truncate">{l.name}</div>
                    <div className="text-fine text-ink-muted-48">
                      {l.chapter_path.join(" / ")} · 类型 {l.leaf_type}
                    </div>
                  </div>
                </label>
              ))}
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
            <Pill className="mt-4" onClick={send} disabled={running || picked.size === 0 || !text}>
              {running ? "发送中…" : `在 ${picked.size} 个节点发送`}
            </Pill>
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
