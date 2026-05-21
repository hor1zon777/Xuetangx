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
};

export type AppSettings = {
  ai: AiSettings;
  heartbeat_interval_ms?: number | null;
  video_speed?: number | null;
  auto_comment_default?: string | null;
  task_concurrency?: number | null;
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
};

export type ExerciseList = {
  exercise_id: number;
  problems: Problem[];
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
  batchExerciseKinds: (sku_id: number, items: [number, number][]) =>
    invoke<Record<string, Record<string, number>>>("batch_exercise_kinds", {
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
  sendComment: (classroom_id: number, leaf_id: number, sign: string, text: string) =>
    invoke<any>("send_comment", { classroomId: classroom_id, leafId: leaf_id, sign, text }),
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
  autoCommentLeaf: (
    classroom_id: number,
    sign: string,
    leaf_ids: number[],
    text: string,
    delay_ms?: number
  ) =>
    invoke<any[]>("auto_comment_leaf", {
      classroomId: classroom_id,
      sign,
      leafIds: leaf_ids,
      text,
      delayMs: delay_ms,
    }),
  listExercise: (exercise_id: number, sku_id: number) =>
    invoke<ExerciseList>("list_exercise", { exerciseId: exercise_id, skuId: sku_id }),
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
  }) => invoke<any[]>("auto_homework_leaf", { args }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<void>("save_settings", { settings }),
  testAi: (settings: AiSettings) => invoke<string>("test_ai", { settings }),
  debugUserCourses: () => invoke<any>("debug_user_courses"),
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

export async function onSettingsUpdated(
  handler: (s: AppSettings) => void
): Promise<UnlistenFn> {
  return await listen("settings://updated", (e) => handler(e.payload as AppSettings));
}
