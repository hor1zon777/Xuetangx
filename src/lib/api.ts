import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Account = {
  user_id: number;
  nickname: string;
  avatar?: string | null;
  login_type?: string | null;
  login_time: number;
  cookies: unknown[];
};

export type Course = {
  classroom_id: number;
  sku_id: number;
  sign: string;
  name: string;
  cover?: string | null;
  status: number;
};

export type LeafNode = {
  id: number;
  name: string;
  leaf_type: number;
  chapter_path: string[];
};

export type VideoTaskStatus = {
  task_id: string;
  leaf_id: number;
  leaf_name?: string | null;
  classroom_id: number;
  current_pos: number;
  duration: number;
  finished: boolean;
  error?: string | null;
  /// 用户主动取消或心跳致命错误终止；前端不应据此标记 leaf 完成
  cancelled?: boolean;
  /// 在等待队列中尚未开始执行
  queued?: boolean;
};

export type AiSettings = {
  base_url: string;
  api_key: string;
  model: string;
  temperature?: number | null;
  system_prompt?: string | null;
  retry_count?: number | null;
  timeout_secs?: number | null;
};

export type AppSettings = {
  ai: AiSettings;
  heartbeat_interval_ms?: number | null;
  video_speed?: number | null;
  auto_comment_default?: string | null;
  task_concurrency?: number | null;
  /** 自动作业是否优先查本地题库（默认 true）。命中后直接组装答案提交，跳过 AI 询问。 */
  use_local_bank?: boolean | null;
  /** 自动作业完成后，是否自动把学堂返回的批改答案入库（默认 true）。 */
  auto_harvest_bank?: boolean | null;
};

export type ProblemKind =
  | "single_choice"
  | "multiple_choice"
  | "completion"
  | "subjective"
  | "judgement"
  | "other";

export type Problem = {
  problem_id: number;
  problem_type: number;
  problem_type_text: string;
  kind: ProblemKind;
  body_html: string;
  options: { key: string; value: string }[];
  /** 是否已批改 */
  submitted?: boolean;
  my_score?: number | null;
  is_right?: boolean | null;
  /** 学堂下发的标准答案（仅 submitted=true 时有效） */
  correct_answer?: string[] | null;
  correct_answer_text?: string | null;
};

export type ExerciseList = {
  exercise_id: number;
  problems: Problem[];
};

export type CaptchaProbe = {
  blocked: boolean;
  captcha?: {
    required: boolean;
    captcha_appid: string;
    exercise_id: number;
    sku_id: number;
    referer?: string | null;
    msg: string;
    error_code: number;
  } | null;
  list?: ExerciseList | null;
};

/**
 * 单个讨论 leaf 的状态：是否评论过 + 题目标题预览。
 *
 * title 来自 `forum/unit/discussion` 响应的 `content.text`：响应里没有真正的
 * `title` 字段，所有"讨论（带分加）"节点在章节树中的 `leaf.name` 又都是
 * 同一个字符串"案例分析"，必须靠 content.text 抽出的预览才能区分。
 */
export type TopicInfo = {
  commented: boolean;
  title: string;
};

/** 题库条目来源：xuetang = 学堂确认答案；manual = 用户手动导入。 */
export type BankSource = "xuetang" | "manual";

export type BankEntry = {
  problem_id: number;
  kind: ProblemKind;
  body_preview: string;
  body_hash: string;
  option_keys: string[];
  answer?: string[] | null;
  answer_text?: string | null;
  source: BankSource;
  updated_at: number;
  hit_count: number;
};

export type BankStats = {
  total: number;
  by_kind: Record<string, number>;
  by_source: Record<string, number>;
  total_hits: number;
};

export type HarvestOutcome = {
  leaf_id: number;
  total_problems: number;
  submitted_problems: number;
  harvested: number;
};

export type BankImportOutcome = {
  added: number;
  updated: number;
  skipped: number;
  total_after: number;
};

export const api = {
  listAccounts: () => invoke<Account[]>("list_accounts"),
  switchAccount: (user_id: number) => invoke<void>("switch_account", { userId: user_id }),
  removeAccount: (user_id: number) => invoke<void>("remove_account", { userId: user_id }),
  currentAccount: () => invoke<Account | null>("current_account"),
  checkLogin: () => invoke<boolean>("check_login"),
  startLogin: () => invoke<void>("start_login"),
  cancelLogin: () => invoke<void>("cancel_login"),
  listCourses: () => invoke<Course[]>("list_courses"),
  listChapters: (classroom_id: number, sign: string) =>
    invoke<LeafNode[]>("list_chapters", { classroomId: classroom_id, sign }),
  leafInfo: (classroom_id: number, leaf_id: number, sign: string) =>
    invoke<any>("leaf_info", { classroomId: classroom_id, leafId: leaf_id, sign }),
  courseSchedule: (classroom_id: number, sign: string) =>
    invoke<Record<string, number>>("course_schedule", {
      classroomId: classroom_id,
      sign,
    }),
  batchExerciseIds: (classroom_id: number, sign: string, leaf_ids: number[]) =>
    invoke<Record<string, number>>("batch_exercise_ids", {
      classroomId: classroom_id,
      sign,
      leafIds: leaf_ids,
    }),
  batchExerciseKinds: (classroom_id: number, sign: string, sku_id: number, items: [number, number][]) =>
    invoke<Record<string, Record<string, number>>>("batch_exercise_kinds", {
      classroomId: classroom_id,
      sign,
      skuId: sku_id,
      items,
    }),
  startVideoTask: (args: {
    classroom_id: number;
    sku_id: number;
    sign: string;
    leaf_id: number;
    override_params?: any;
    speed?: number;
    interval_ms?: number;
    start_position?: number;
    leaf_name?: string;
  }) => invoke<VideoTaskStatus>("start_video_task", { args }),
  stopVideoTask: (task_id: string) => invoke<void>("stop_video_task", { taskId: task_id }),
  listVideoTasks: () => invoke<VideoTaskStatus[]>("list_video_tasks"),
  sendComment: (
    classroom_id: number,
    leaf_id: number,
    sign: string,
    text: string,
    topic_type?: number
  ) =>
    invoke<any>("send_comment", {
      classroomId: classroom_id,
      leafId: leaf_id,
      sign,
      text,
      topicType: topic_type,
    }),
  listTopicComments: (
    topic_id: number,
    classroom_id: number,
    leaf_id: number,
    offset = 0,
    limit = 10
  ) =>
    invoke<any>("list_topic_comments", {
      topicId: topic_id,
      classroomId: classroom_id,
      leafId: leaf_id,
      offset,
      limit,
    }),
  /**
   * 批量在 leaf 下发评论。
   * `topic_type` 默认 0（视频底下旧讨论）；leaf_type=4 的"讨论（带分加）"节点要传 4，
   * 并把 `report_schedule=true` + `sku_id` 一起传过来，让后端在评论成功后上报
   * chapter/schedule 触发该节点的"已完成"。
   */
  autoCommentLeaf: (
    classroom_id: number,
    sign: string,
    leaf_ids: number[],
    text: string,
    options?: {
      delay_ms?: number;
      topic_type?: number;
      report_schedule?: boolean;
      sku_id?: number;
    }
  ) =>
    invoke<any[]>("auto_comment_leaf", {
      classroomId: classroom_id,
      sign,
      leafIds: leaf_ids,
      text,
      delayMs: options?.delay_ms,
      topicType: options?.topic_type,
      reportSchedule: options?.report_schedule,
      skuId: options?.sku_id,
    }),
  /**
   * 批量"标记图文已学完"（leaf_type=3 节点）。
   * 对每个 leaf 顺序调用 user_article_finish + POST chapter/schedule。
   */
  autoArticleLeaf: (
    classroom_id: number,
    sku_id: number,
    sign: string,
    leaf_ids: number[],
    delay_ms?: number
  ) =>
    invoke<any[]>("auto_article_leaf", {
      classroomId: classroom_id,
      skuId: sku_id,
      sign,
      leafIds: leaf_ids,
      delayMs: delay_ms,
    }),
  /**
   * 批量检测每个 leaf 的讨论区中"当前账号是否发过评论"，同时拿到该节点的题目标题。
   *
   * 返回 `{ [leaf_id]: { commented, title } }`：
   * - commented：当前账号是否在该节点评论过
   * - title：从 forum/unit/discussion 响应的 content.text 中提取的预览（前 ~40 字），
   *   用于解决"所有讨论节点 leaf.name 都叫 '案例分析' 区分不开"的问题。
   *
   * 未出现在结果里的 leaf 表示请求失败（视作未知）。
   * `topic_type` 默认 0；检测带分加讨论需传 4。
   */
  batchMyCommentStatus: (
    classroom_id: number,
    sign: string,
    leaf_ids: number[],
    topic_type?: number
  ) =>
    invoke<Record<string, TopicInfo>>("batch_my_comment_status", {
      classroomId: classroom_id,
      sign,
      leafIds: leaf_ids,
      topicType: topic_type,
    }),
  listExercise: (exercise_id: number, sku_id: number) =>
    invoke<ExerciseList>("list_exercise", {
      exerciseId: exercise_id,
      skuId: sku_id,
    }),
  listExerciseWithCaptcha: (
    exercise_id: number,
    sku_id: number,
    referer?: string,
    ticket?: string,
    randstr?: string
  ) =>
    invoke<ExerciseList>("list_exercise_with_captcha", {
      exerciseId: exercise_id,
      skuId: sku_id,
      referer,
      ticket,
      randstr,
    }),
  probeExerciseCaptcha: (exercise_id: number, sku_id: number, referer?: string) =>
    invoke<CaptchaProbe>("probe_exercise_captcha", {
      exerciseId: exercise_id,
      skuId: sku_id,
      referer,
    }),
  submitProblem: (args: {
    leaf_id: number;
    classroom_id: number;
    exercise_id: number;
    problem_id: number;
    sign: string;
    answer: string[];
    answers?: any;
  }) => invoke<any>("submit_problem", { args }),
  autoHomeworkLeaf: (args: {
    leaf_id: number;
    classroom_id: number;
    sku_id: number;
    exercise_id: number;
    sign: string;
    ticket?: string;
    randstr?: string;
  }) => invoke<any[]>("auto_homework_leaf", { args }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<void>("save_settings", { settings }),
  testAi: (settings: AiSettings) => invoke<string>("test_ai", { settings }),
  debugUserCourses: () => invoke<any>("debug_user_courses"),
  /**
   * 单节点题库收录：拉一次 get_exercise_list，把已批改题目入库。**不会触发任何提交**。
   * 仅在用户已经完成过该节点的作业、想把答案沉淀到本地时调用。
   */
  harvestExerciseAnswers: (args: {
    leaf_id: number;
    classroom_id: number;
    sku_id: number;
    exercise_id: number;
    sign: string;
  }) => invoke<HarvestOutcome>("harvest_exercise_answers", { args }),
  /**
   * 批量题库收录：按 ≤ 1 req/s 顺序处理每个节点。进度通过 `bank://progress` 事件流式上报。
   */
  batchHarvestCourseAnswers: (args: {
    classroom_id: number;
    sku_id: number;
    sign: string;
    leaves: { leaf_id: number; exercise_id: number }[];
  }) =>
    invoke<{
      total: number;
      total_submitted: number;
      total_harvested: number;
      items: any[];
    }>("batch_harvest_course_answers", { args }),
  bankList: (keyword?: string, offset = 0, limit = 200) =>
    invoke<BankEntry[]>("bank_list", { args: { keyword, offset, limit } }),
  bankGet: (problem_id: number) =>
    invoke<BankEntry | null>("bank_get", { problemId: problem_id }),
  bankDelete: (problem_id: number) =>
    invoke<boolean>("bank_delete", { problemId: problem_id }),
  bankClear: () => invoke<number>("bank_clear"),
  bankExport: () => invoke<BankEntry[]>("bank_export"),
  bankImport: (entries: BankEntry[]) =>
    invoke<BankImportOutcome>("bank_import", { args: { entries } }),
  bankStats: () => invoke<BankStats>("bank_stats"),
};

export async function onLoginEvents(handlers: {
  onQr?: (p: { ticket: string; loginid: number; expire_seconds: number }) => void;
  onScanned?: (p: { user_id: number }) => void;
  onSuccess?: (p: { user_id: number; nickname: string }) => void;
  onError?: (p: { message: string }) => void;
  onCancelled?: () => void;
}): Promise<UnlistenFn> {
  const unlisteners: UnlistenFn[] = [];
  if (handlers.onQr) unlisteners.push(await listen("login://qr", (e) => handlers.onQr!(e.payload as any)));
  if (handlers.onScanned)
    unlisteners.push(await listen("login://scanned", (e) => handlers.onScanned!(e.payload as any)));
  if (handlers.onSuccess)
    unlisteners.push(await listen("login://success", (e) => handlers.onSuccess!(e.payload as any)));
  if (handlers.onError)
    unlisteners.push(await listen("login://error", (e) => handlers.onError!(e.payload as any)));
  if (handlers.onCancelled)
    unlisteners.push(await listen("login://cancelled", () => handlers.onCancelled!()));
  return () => unlisteners.forEach((u) => u());
}

export async function onVideoEvents(handlers: {
  onProgress?: (p: VideoTaskStatus) => void;
  onDone?: (p: VideoTaskStatus) => void;
  onError?: (p: { task_id: string; message: string }) => void;
}): Promise<UnlistenFn> {
  const unlisteners: UnlistenFn[] = [];
  if (handlers.onProgress)
    unlisteners.push(await listen("video://progress", (e) => handlers.onProgress!(e.payload as any)));
  if (handlers.onDone)
    unlisteners.push(await listen("video://done", (e) => handlers.onDone!(e.payload as any)));
  if (handlers.onError)
    unlisteners.push(await listen("video://error", (e) => handlers.onError!(e.payload as any)));
  return () => unlisteners.forEach((u) => u());
}

/**
 * 批量评论后端进度事件。
 * - throttle    : 稳态间隔等待中，extra.wait_ms = 还需等待毫秒
 * - sending     : 正在发送某一条，extra.attempt = 第几次尝试（0 = 首次）
 * - rate_limited: 命中 429，extra.retry_after_s = 服务端要求等待秒数，attempt = 已重试次数
 * - item        : 某一条彻底结束（成功 / 失败），extra = 结果对象 { leaf_id, ok, data?, error? }
 * - done        : 整批结束
 */
export type ForumProgress = {
  phase: "throttle" | "sending" | "rate_limited" | "item" | "done";
  index: number;
  total: number;
  interval_ms: number;
  extra: Record<string, any>;
};

export async function onForumProgress(
  handler: (p: ForumProgress) => void
): Promise<UnlistenFn> {
  return await listen("forum://progress", (e) => handler(e.payload as ForumProgress));
}

/**
 * 批量"图文标记完成"事件。结构比评论简洁，没有限速重试：
 * - throttle: 稳态间隔等待中，extra.wait_ms = 还需等待毫秒
 * - sending : 正在 POST chapter/schedule
 * - item    : 单条结束（成功 / 失败），extra = { leaf_id, ok, data? | error? }
 * - done    : 整批结束
 */
export type ArticleProgress = {
  phase: "throttle" | "sending" | "item" | "done";
  index: number;
  total: number;
  interval_ms: number;
  extra: Record<string, any>;
};

export async function onArticleProgress(
  handler: (p: ArticleProgress) => void
): Promise<UnlistenFn> {
  return await listen("article://progress", (e) =>
    handler(e.payload as ArticleProgress)
  );
}

/**
 * 自动作业的逐题进度事件。
 * - start     : 进入下一题，info = { problem_id, kind, kind_label, index, total }
 * - asking_ai : 正在询问 AI
 * - submitting: 正在提交答案，info.answer_text 是 AI 给的应答
 * - skipped   : 学堂已批改，本次跳过
 * - bank_hit  : 本地题库命中，info.answer_text 是本地答案，info.matched_by = "problem_id" | "body_hash"
 * - item_done : 单题彻底结束，info.result 是回传给前端的完整记录
 * - done      : 整套题目结束，info.bank_harvested = 本批次自动入库的题数
 */
export type HomeworkProgress = {
  leaf_id: number;
  phase:
    | "start"
    | "asking_ai"
    | "submitting"
    | "skipped"
    | "bank_hit"
    | "item_done"
    | "done";
  info: {
    problem_id?: number;
    kind?: string;
    kind_label?: string;
    index?: number;
    total?: number;
    my_score?: number | null;
    is_right?: boolean | null;
    answer_text?: string;
    matched_by?: "problem_id" | "body_hash";
    source_problem_id?: number;
    from_bank?: boolean;
    bank_harvested?: number;
    result?: any;
  };
};

export async function onHomeworkProgress(
  handler: (p: HomeworkProgress) => void
): Promise<UnlistenFn> {
  return await listen("homework://progress", (e) =>
    handler(e.payload as HomeworkProgress)
  );
}

export async function onSettingsUpdated(
  handler: (s: AppSettings) => void
): Promise<UnlistenFn> {
  return await listen("settings://updated", (e) => handler(e.payload as AppSettings));
}

/**
 * 批量题库收录的进度事件。
 * - throttle : 限速等待，extra.wait_ms = 还需等待毫秒
 * - fetching : 正在拉取某个 leaf 的习题列表
 * - item     : 单条结束，extra = { leaf_id, ok, total?, submitted?, harvested? | error? }
 * - done     : 整批结束，extra = { total_submitted, total_harvested }
 */
export type BankProgress = {
  phase: "throttle" | "fetching" | "item" | "done";
  index: number;
  total: number;
  interval_ms: number;
  extra: Record<string, any>;
};

export async function onBankProgress(
  handler: (p: BankProgress) => void
): Promise<UnlistenFn> {
  return await listen("bank://progress", (e) => handler(e.payload as BankProgress));
}
