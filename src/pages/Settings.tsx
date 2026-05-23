import { useEffect, useState } from "react";
import { api, type AppSettings } from "../lib/api";
import { Card, Field, Pill, SectionTitle } from "../components/ui";
import { toast } from "../components/Toast";

const DEFAULT: AppSettings = {
  ai: {
    base_url: "https://api.openai.com/v1",
    api_key: "",
    model: "gpt-4o-mini",
    temperature: 0.1,
    system_prompt: "你是一位严谨的中文学科助教，只输出最终答案，不要解释。",
    retry_count: 2,
    timeout_secs: 30,
  },
  heartbeat_interval_ms: 5000,
  video_speed: 1.0,
  auto_comment_default: "学到了，感谢老师。",
  task_concurrency: 3,
  use_local_bank: true,
  auto_harvest_bank: true,
};

export function SettingsPage() {
  const [s, setS] = useState<AppSettings>(DEFAULT);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string | null>(null);

  useEffect(() => {
    api.getSettings().then((v) => {
      setS({ ...DEFAULT, ...v, ai: { ...DEFAULT.ai, ...(v?.ai || {}) } });
    });
  }, []);

  const save = async () => {
    setSaving(true);
    try {
      await api.saveSettings(s);
      toast.success("设置已保存");
    } catch (e: any) {
      toast.error(`保存失败：${e}`);
    } finally {
      setSaving(false);
    }
  };

  const test = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const r = await api.testAi(s.ai);
      setTestResult(r);
    } catch (e: any) {
      setTestResult(`失败：${e}`);
    } finally {
      setTesting(false);
    }
  };

  return (
    <div>
      <SectionTitle
        title="设置"
        subtitle="配置 OpenAI 兼容大模型与刷课参数。所有设置仅保存在本地。"
      />
      <div className="px-12 grid grid-cols-1 lg:grid-cols-2 gap-6">
        <Card>
          <div className="font-display text-tagline mb-4">大模型（OpenAI 兼容）</div>
          <div className="space-y-4">
            <Field label="API Base URL" hint="例如 https://api.openai.com/v1 或自建反代地址">
              <input
                className="field"
                value={s.ai.base_url}
                onChange={(e) => setS({ ...s, ai: { ...s.ai, base_url: e.target.value } })}
              />
            </Field>
            <Field label="API Key">
              <input
                className="field"
                type="password"
                value={s.ai.api_key}
                onChange={(e) => setS({ ...s, ai: { ...s.ai, api_key: e.target.value } })}
              />
            </Field>
            <Field label="模型名">
              <input
                className="field"
                value={s.ai.model}
                onChange={(e) => setS({ ...s, ai: { ...s.ai, model: e.target.value } })}
              />
            </Field>
            <Field label="Temperature">
              <input
                className="field"
                type="number"
                step="0.1"
                value={s.ai.temperature ?? 0.1}
                onChange={(e) =>
                  setS({ ...s, ai: { ...s.ai, temperature: Number(e.target.value) } })
                }
              />
            </Field>
            <Field label="AI 询问失败重试次数" hint="额外重试次数；0 表示失败后不重试">
              <input
                className="field"
                type="number"
                min={0}
                step={1}
                value={s.ai.retry_count ?? 2}
                onChange={(e) =>
                  setS({
                    ...s,
                    ai: {
                      ...s.ai,
                      retry_count: Math.max(0, Math.trunc(Number(e.target.value) || 0)),
                    },
                  })
                }
              />
            </Field>
            <Field label="AI 单次请求超时（秒）" hint="每次询问大模型最多等待多久，默认 30 秒">
              <input
                className="field"
                type="number"
                min={1}
                step={1}
                value={s.ai.timeout_secs ?? 30}
                onChange={(e) =>
                  setS({
                    ...s,
                    ai: {
                      ...s.ai,
                      timeout_secs: Math.max(1, Math.trunc(Number(e.target.value) || 30)),
                    },
                  })
                }
              />
            </Field>
            <Field label="System Prompt">
              <textarea
                className="field min-h-[80px]"
                value={s.ai.system_prompt ?? ""}
                onChange={(e) =>
                  setS({ ...s, ai: { ...s.ai, system_prompt: e.target.value } })
                }
              />
            </Field>
            <div className="flex gap-3">
              <Pill onClick={test} disabled={testing}>
                {testing ? "测试中…" : "测试连接"}
              </Pill>
              {testResult && (
                <div className="text-caption text-ink-muted-80 self-center break-all">
                  {testResult}
                </div>
              )}
            </div>
          </div>
        </Card>
        <Card>
          <div className="font-display text-tagline mb-4">刷课参数</div>
          <div className="space-y-4">
            <Field label="心跳间隔（毫秒）" hint="学堂在线默认 5000ms，不建议低于 3000ms">
              <input
                className="field"
                type="number"
                value={s.heartbeat_interval_ms ?? 5000}
                onChange={(e) =>
                  setS({ ...s, heartbeat_interval_ms: Number(e.target.value) })
                }
              />
            </Field>
            <Field label="默认播放倍速" hint="范围 0.5–2.0">
              <input
                className="field"
                type="number"
                step="0.1"
                min={0.5}
                max={2}
                value={s.video_speed ?? 1}
                onChange={(e) => setS({ ...s, video_speed: Number(e.target.value) })}
              />
            </Field>
            <Field label="默认评论文本">
              <input
                className="field"
                value={s.auto_comment_default ?? ""}
                onChange={(e) =>
                  setS({ ...s, auto_comment_default: e.target.value })
                }
              />
            </Field>
            <Field label="最大并发任务数" hint="同时刷课的最大任务数，留空表示不限制（最小 1）">
              <input
                className="field"
                type="number"
                min={1}
                value={s.task_concurrency ?? ""}
                onChange={(e) => {
                  const v = e.target.value;
                  setS({
                    ...s,
                    task_concurrency: v === "" ? null : Math.max(1, Number(v)),
                  });
                }}
                placeholder="不限制"
              />
            </Field>
          </div>
        </Card>
        <Card>
          <div className="font-display text-tagline mb-4">自动作业偏好</div>
          <div className="space-y-4">
            <label className="flex items-start gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={s.use_local_bank ?? true}
                onChange={(e) => setS({ ...s, use_local_bank: e.target.checked })}
                className="mt-1"
              />
              <div className="flex-1">
                <div className="text-body text-ink">优先使用本地题库</div>
                <div className="text-fine text-ink-muted-48 mt-0.5 leading-relaxed">
                  自动作业时先查本地题库，命中后直接提交、跳过 AI 询问。命中只发生在
                  题库里有过同一道题（按 problem_id 或题面 + 选项哈希匹配）的情况，
                  来源是学堂已批改答案，绝对可信。
                </div>
              </div>
            </label>
            <label className="flex items-start gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={s.auto_harvest_bank ?? true}
                onChange={(e) => setS({ ...s, auto_harvest_bank: e.target.checked })}
                className="mt-1"
              />
              <div className="flex-1">
                <div className="text-body text-ink">自动收录答案</div>
                <div className="text-fine text-ink-muted-48 mt-0.5 leading-relaxed">
                  每次自动作业完成后，再拉一次习题列表，把刚被学堂批改的标准答案
                  写入本地题库。仅写入"学堂确认"的答案，AI 给的答案不会写入。
                </div>
              </div>
            </label>
          </div>
        </Card>
      </div>
      <div className="px-12 mt-6">
        <Pill onClick={save} disabled={saving}>
          {saving ? "保存中…" : "保存设置"}
        </Pill>
      </div>
    </div>
  );
}
