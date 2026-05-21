import { useEffect, useState } from "react";
import { api, type Account, type Course } from "../lib/api";
import { Card, Pill, SectionTitle, Spinner } from "../components/ui";

export function HomePage({ current }: { current: Account | null }) {
  const [courses, setCourses] = useState<Course[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [debug, setDebug] = useState<any>(null);

  const load = async () => {
    if (!current) return;
    setLoading(true);
    setError(null);
    try {
      setCourses(await api.listCourses());
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const runDebug = async () => {
    setDebug(null);
    try {
      setDebug(await api.debugUserCourses());
    } catch (e: any) {
      setDebug({ error: String(e) });
    }
  };

  useEffect(() => {
    load();
  }, [current?.user_id]);

  return (
    <div>
      <SectionTitle
        title={`你好，${current?.nickname || "同学"}`}
        subtitle="选择课程开始自动学习，或前往左侧功能区。"
      />
      <div className="px-12 flex gap-3 mb-4 flex-wrap">
        <Pill onClick={load} disabled={loading}>
          {loading ? <Spinner /> : "刷新课程列表"}
        </Pill>
        <Pill variant="ghost" onClick={runDebug}>
          调试接口
        </Pill>
        {error && <span className="text-caption text-[#cc2b2b] self-center">{error}</span>}
      </div>
      <div className="px-12 grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-6">
        {courses.map((c) => (
          <Card key={c.classroom_id}>
            {c.cover && (
              <img
                src={c.cover}
                referrerPolicy="no-referrer"
                crossOrigin="anonymous"
                onError={(e) => {
                  (e.currentTarget as HTMLImageElement).style.display = "none";
                }}
                className="w-full aspect-video object-cover rounded-sm mb-3"
                alt=""
              />
            )}
            <div className="font-display text-tagline text-ink line-clamp-2 min-h-[2.4em]">
              {c.name}
            </div>
            <div className="text-caption text-ink-muted-48 mt-1">
              classroom_id：{c.classroom_id} · sku：{c.sku_id} · status：{c.status}
            </div>
            <div className="text-fine text-ink-muted-48 mt-1 break-all">sign：{c.sign}</div>
          </Card>
        ))}
        {!loading && courses.length === 0 && (
          <Card>
            <div className="text-body text-ink-muted-80">
              暂无课程。请确认账号已选课，或点击「调试接口」查看接口实际返回。
            </div>
          </Card>
        )}
      </div>
      {debug && (
        <div className="px-12 mt-6">
          <Card>
            <div className="font-display text-tagline mb-2">接口诊断</div>
            <pre className="text-fine text-ink-muted-80 overflow-auto max-h-[420px] whitespace-pre-wrap break-all">
              {JSON.stringify(debug, null, 2)}
            </pre>
          </Card>
        </div>
      )}
    </div>
  );
}
