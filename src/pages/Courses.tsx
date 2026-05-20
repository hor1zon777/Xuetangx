import { useEffect, useState } from "react";
import { api, type Account, type Course } from "../lib/api";
import { Capsule, Card, Pill, SectionTitle, Spinner } from "../components/ui";

export function CoursesPage({ current }: { current: Account | null }) {
  const [list, setList] = useState<Course[]>([]);
  const [loading, setLoading] = useState(false);

  const load = async () => {
    if (!current) return;
    setLoading(true);
    try {
      setList(await api.listCourses());
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, [current?.user_id]);

  return (
    <div>
      <SectionTitle title="我的课程" subtitle="检查课程基本信息，确认 classroom_id / sign / sku_id。" />
      <div className="px-12 flex gap-3 mb-4">
        <Pill onClick={load}>{loading ? <Spinner /> : "刷新"}</Pill>
      </div>
      <div className="px-12 grid grid-cols-1 md:grid-cols-2 gap-6">
        {list.map((c) => (
          <Card key={c.classroom_id}>
            <div className="font-display text-tagline">{c.name}</div>
            <div className="text-caption text-ink-muted-80 mt-2 space-y-1">
              <div>classroom_id：{c.classroom_id}</div>
              <div>sku_id：{c.sku_id}</div>
              <div className="break-all">sign：{c.sign}</div>
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
}
