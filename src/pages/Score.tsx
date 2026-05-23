import { useEffect, useMemo, useState } from "react";
import {
  api,
  type Course,
  type EvaluationDetail,
  type EvaluationCategory,
} from "../lib/api";
import { Capsule, Card, SectionTitle, Spinner } from "../components/ui";
import { RefreshIcon } from "../components/icons";
import { toast } from "../components/Toast";

/**
 * 总成绩页：复用学堂在线官网的 `get_evaluation_detail` 接口，一次拿到
 * 真实总分、等级、5 个评分大类（视频/图文/讨论/作业/考试）的占比与得分，
 * 以及每个计分 leaf 的实得分数。无需启发式估算。
 *
 * UI 布局参考学堂在线网页的进度页：
 *   - 顶部：当前总分 + 等级徽章 + 距下一级差
 *   - 中部：评分规则说明（"满分 100 = 视频 X% + 图文 X% + …"）
 *   - 下部：5 个分项 Card，每个含完成度条、当前得分、leaf 明细列表
 */
export function ScorePage() {
  const [courses, setCourses] = useState<Course[]>([]);
  const [selected, setSelected] = useState<Course | null>(null);
  const [loading, setLoading] = useState(false);
  const [detail, setDetail] = useState<EvaluationDetail | null>(null);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  // 首次进入懒拉课程列表。和 Homework 页一样的写法。
  useEffect(() => {
    api.listCourses().then(setCourses);
  }, []);

  const loadDetail = async (c: Course) => {
    setSelected(c);
    setDetail(null);
    setLoading(true);
    setExpanded(new Set());
    try {
      const d = await api.courseEvaluationDetail(c.classroom_id, c.sign);
      setDetail(d);
    } catch (e: any) {
      toast.error(`拉取成绩失败：${e}`);
    } finally {
      setLoading(false);
    }
  };

  const refresh = async () => {
    if (!selected) return;
    await loadDetail(selected);
    toast.success("成绩已刷新");
  };

  const toggleCategory = (id: number) => {
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div>
      <SectionTitle
        title="总成绩"
        subtitle="拉取学堂在线进度页的真实数据：当前总分、等级、各评分项已得分与完成度。"
      />
      <div className="px-12 flex gap-3 flex-wrap mb-4">
        {courses.map((c) => (
          <Capsule
            key={c.classroom_id}
            selected={selected?.classroom_id === c.classroom_id}
            onClick={() => loadDetail(c)}
          >
            {c.name}
          </Capsule>
        ))}
        {courses.length === 0 && (
          <span className="text-caption text-ink-muted-48">
            还没有可选课程，先去"我的课程"加载一下。
          </span>
        )}
      </div>

      {loading && (
        <div className="px-12 flex items-center gap-2 text-body text-ink-muted-80">
          <Spinner />
          <span>正在拉取成绩明细…</span>
        </div>
      )}

      {selected && detail && !loading && (
        <div className="px-12 grid grid-cols-1 lg:grid-cols-3 gap-6">
          <TotalScoreCard detail={detail} onRefresh={refresh} />
          <ScoringRuleCard categories={detail.categories} />
          <div className="lg:col-span-3 space-y-3">
            <div className="text-body text-ink-muted-80">
              成绩明细（满分 100 分）
            </div>
            {detail.categories.map((cat) => (
              <CategoryRow
                key={cat.evaluation_id}
                cat={cat}
                expanded={expanded.has(cat.evaluation_id)}
                onToggle={() => toggleCategory(cat.evaluation_id)}
              />
            ))}
            {detail.categories.length === 0 && (
              <div className="text-body text-ink-muted-48">
                本课程暂无评分项明细。
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function TotalScoreCard({
  detail,
  onRefresh,
}: {
  detail: EvaluationDetail;
  onRefresh: () => void;
}) {
  const t = detail.total;
  const passLine =
    t.pass_line !== null && t.pass_line !== undefined ? t.pass_line : null;
  const passed = passLine !== null ? t.user_score >= passLine : null;
  return (
    <Card>
      <div className="flex items-start justify-between gap-3 mb-3">
        <div className="font-display text-tagline">当前得分</div>
        <button
          className="text-link text-caption inline-flex items-center gap-1"
          onClick={onRefresh}
          title="重新拉取成绩明细"
        >
          <RefreshIcon className="w-3.5 h-3.5" />
          刷新
        </button>
      </div>
      <div className="flex items-baseline gap-2 mb-1">
        <span className="font-display text-display-lg text-ink">
          {t.user_score.toFixed(2)}
        </span>
        <span className="text-body text-ink-muted-48">/ 100</span>
      </div>
      <div className="flex items-center gap-2 mb-3">
        <TitleBadge title={t.title} />
        {passLine !== null && (
          <span
            className={`text-fine inline-flex items-center px-2 h-[20px] rounded-pill ${
              passed
                ? "text-action-blue bg-action-blue/10"
                : "text-[#cc2b2b] bg-[#cc2b2b]/10"
            }`}
          >
            及格线 {passLine} · {passed ? "已及格" : "未及格"}
          </span>
        )}
      </div>
      {t.higher_title && t.lack_score > 0 && (
        <div className="text-fine text-ink-muted-80 leading-relaxed">
          距离下个等级 <strong>{t.higher_title}</strong> 还需要{" "}
          <strong>{t.lack_score.toFixed(2)}</strong> 分，继续加油～
        </div>
      )}
      {t.higher_title === "" && (
        <div className="text-fine text-ink-muted-48 leading-relaxed">
          已经是最高等级。
        </div>
      )}
    </Card>
  );
}

function ScoringRuleCard({ categories }: { categories: EvaluationCategory[] }) {
  if (categories.length === 0) return <Card>暂无评分规则。</Card>;
  const rule = categories
    .map((c) => `${c.evaluation_name}(${c.proportion}%)`)
    .join(" + ");
  return (
    <Card className="lg:col-span-2">
      <div className="font-display text-tagline mb-3">如何记分？</div>
      <div className="text-body text-ink leading-relaxed mb-2">
        最终成绩按照考核的百分制计分，满分为 100 分。
      </div>
      <div className="text-body text-ink-muted-80 leading-relaxed mb-3">
        满分 100 分 = {rule}
      </div>
      <div className="text-fine text-ink-muted-48 leading-relaxed">
        学习者完成加入考核的学习单元，即可得到相应分数。未被加入考核的学习单元，
        学习后不计算成绩。
      </div>
    </Card>
  );
}

function CategoryRow({
  cat,
  expanded,
  onToggle,
}: {
  cat: EvaluationCategory;
  expanded: boolean;
  onToggle: () => void;
}) {
  // 完成度按 0~1 渲染条；超过 1 容错为 100%。
  const pct = Math.max(0, Math.min(1, cat.schedule)) * 100;
  return (
    <Card>
      <button
        type="button"
        onClick={onToggle}
        className="w-full text-left flex items-center justify-between gap-4"
      >
        <div className="flex-1 min-w-0">
          <div className="flex items-baseline gap-3 mb-1">
            <span className="text-body text-ink font-medium">
              {cat.evaluation_name}
            </span>
            <span className="text-fine text-ink-muted-48">
              {cat.evaluation_score.toFixed(1)} 分（占总分 {cat.proportion}%）
            </span>
          </div>
          <div className="flex items-center gap-3">
            <div className="flex-1 h-1.5 bg-ink-muted-12 rounded-pill overflow-hidden">
              <div
                className="h-full bg-action-blue rounded-pill"
                style={{ width: `${pct}%` }}
              />
            </div>
            <span className="text-fine text-ink-muted-80 whitespace-nowrap">
              已完成 {pct.toFixed(0)}%
            </span>
            <span className="text-fine text-ink whitespace-nowrap">
              得分 {cat.use_evaluation_score.toFixed(2)} /{" "}
              {cat.evaluation_score.toFixed(1)}
            </span>
          </div>
        </div>
        <div className="text-caption text-ink-muted-48 select-none">
          {expanded ? "收起 ▴" : `展开 ▾ (${cat.leaves.length})`}
        </div>
      </button>

      {expanded && cat.leaves.length > 0 && (
        <div className="mt-3 pt-3 border-t border-divider-soft divide-y divide-divider-soft">
          {cat.leaves.map((leaf) => {
            const lpct = Math.max(0, Math.min(1, leaf.schedule)) * 100;
            const done = leaf.schedule >= 1;
            return (
              <div
                key={leaf.leaf_id}
                className="py-2 flex items-start gap-3"
              >
                <div className="flex-1 min-w-0">
                  <div className="text-body truncate">{leaf.leaf_name}</div>
                  {leaf.chapter_path.length > 0 && (
                    <div className="text-fine text-ink-muted-48 truncate">
                      {leaf.chapter_path.join(" / ")}
                    </div>
                  )}
                </div>
                <div className="flex items-center gap-3 text-fine whitespace-nowrap">
                  <span
                    className={
                      done ? "text-action-blue" : "text-ink-muted-80"
                    }
                  >
                    {done ? "已完成" : `${lpct.toFixed(0)}%`}
                  </span>
                  <span className="text-ink">
                    {leaf.user_score.toFixed(2)} / {leaf.leaf_score.toFixed(2)}
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      )}
      {expanded && cat.leaves.length === 0 && (
        <div className="mt-3 pt-3 border-t border-divider-soft text-fine text-ink-muted-48">
          该分类下无可见的计分节点。
        </div>
      )}
    </Card>
  );
}

/**
 * 等级徽章。学堂在线常见的等级有 F / D / C / B / A / P，颜色按分数高低区分：
 * F 红、D 橙、C 黄、B 蓝绿、A 绿、P 紫。其它（空串等）走中性灰。
 */
function TitleBadge({ title }: { title: string }) {
  const color = useMemo(() => {
    switch (title) {
      case "A":
      case "P":
        return "bg-[#1e8f4e] text-white";
      case "B":
        return "bg-action-blue text-white";
      case "C":
        return "bg-[#d6a200] text-white";
      case "D":
        return "bg-[#d97706] text-white";
      case "F":
        return "bg-[#cc2b2b] text-white";
      default:
        return "bg-ink/80 text-white";
    }
  }, [title]);
  if (!title) return null;
  return (
    <span
      className={`inline-flex items-center justify-center w-7 h-7 rounded-pill font-display text-caption ${color}`}
    >
      {title}
    </span>
  );
}
