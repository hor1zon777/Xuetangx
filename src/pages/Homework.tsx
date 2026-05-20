import { useEffect, useMemo, useState } from "react";
import { api, type Course, type LeafNode } from "../lib/api";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";

type LeafExtra = {
  exercise_id?: number;
};

export function HomeworkPage() {
  const [courses, setCourses] = useState<Course[]>([]);
  const [selected, setSelected] = useState<Course | null>(null);
  const [leaves, setLeaves] = useState<LeafNode[]>([]);
  const [extra, setExtra] = useState<Record<number, LeafExtra>>({});
  const [loading, setLoading] = useState(false);
  const [running, setRunning] = useState(false);
  const [results, setResults] = useState<any[]>([]);

  const homeworkLeaves = useMemo(
    () => leaves.filter((l) => l.leaf_type === 6 || l.leaf_type === 7 || l.leaf_type === 3),
    [leaves]
  );

  useEffect(() => {
    api.listCourses().then(setCourses);
  }, []);

  const loadLeaves = async (c: Course) => {
    setSelected(c);
    setLoading(true);
    setExtra({});
    try {
      const ls = await api.listChapters(c.classroom_id, c.sign);
      setLeaves(ls);
      // 异步预取 exercise_id（leaf_info.content.exercise_id 或 .exercise_id）
      for (const l of ls.filter((x) => x.leaf_type === 6 || x.leaf_type === 7 || x.leaf_type === 3)) {
        api
          .leafInfo(c.classroom_id, l.id, c.sign)
          .then((info) => {
            const exId =
              info?.content?.exercise_id ??
              info?.exercise_id ??
              info?.content?.id ??
              null;
            if (exId)
              setExtra((m) => ({ ...m, [l.id]: { exercise_id: Number(exId) } }));
          })
          .catch(() => {});
      }
    } finally {
      setLoading(false);
    }
  };

  const run = async (l: LeafNode) => {
    if (!selected) return;
    const exId = extra[l.id]?.exercise_id;
    if (!exId) {
      alert("尚未拿到该节点的 exercise_id，请稍候或手动确认这是习题节点。");
      return;
    }
    setRunning(true);
    setResults([]);
    try {
      const out = await api.autoHomeworkLeaf({
        leaf_id: l.id,
        classroom_id: selected.classroom_id,
        sku_id: selected.sku_id,
        exercise_id: exId,
        sign: selected.sign,
      });
      setResults(out);
    } catch (e: any) {
      setResults([{ error: String(e) }]);
    } finally {
      setRunning(false);
    }
  };

  return (
    <div>
      <SectionTitle
        title="自动作业"
        subtitle="拉取题目 → 询问大模型 → 自动提交。请先在设置中配置 OpenAI 兼容 API。"
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
              <div className="font-display text-tagline">习题节点</div>
              {loading && <Spinner />}
            </div>
            <div className="max-h-[420px] overflow-auto divide-y divide-divider-soft">
              {homeworkLeaves.map((l) => (
                <div key={l.id} className="flex items-center gap-3 py-2">
                  <div className="flex-1 min-w-0">
                    <div className="text-body truncate">{l.name}</div>
                    <div className="text-fine text-ink-muted-48">
                      {l.chapter_path.join(" / ")} · 类型 {l.leaf_type}
                    </div>
                  </div>
                  <span className="text-fine text-ink-muted-48 min-w-[110px] text-right">
                    {extra[l.id]?.exercise_id
                      ? `exercise: ${extra[l.id]!.exercise_id}`
                      : "解析中…"}
                  </span>
                  <Pill
                    variant="ghost"
                    onClick={() => run(l)}
                    disabled={!extra[l.id]?.exercise_id || running}
                  >
                    开始
                  </Pill>
                </div>
              ))}
              {!loading && homeworkLeaves.length === 0 && (
                <div className="text-body text-ink-muted-80 py-6">本课程无习题节点。</div>
              )}
            </div>
          </Card>
          <Card>
            <div className="font-display text-tagline mb-3">执行结果</div>
            <div className="max-h-[460px] overflow-auto space-y-2">
              {results.map((r, i) => (
                <div
                  key={i}
                  className={`text-caption ${
                    r.error || r.submit?.is_right === false
                      ? "text-[#cc2b2b]"
                      : "text-action-blue"
                  }`}
                >
                  题目 {r.problem_id ?? "-"} · 答案 {(r.answer || []).join("")}
                  {r.error ? ` · 错误：${r.error}` : ""}
                  {r.submit
                    ? ` · ${r.submit.is_right ? "正确" : "未通过"}（得分 ${
                        r.submit.my_score ?? "?"
                      }）`
                    : ""}
                </div>
              ))}
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
