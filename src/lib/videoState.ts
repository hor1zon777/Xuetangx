import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  onSettingsUpdated,
  onVideoEvents,
  type Course,
  type LeafNode,
  type VideoTaskStatus,
} from "./api";

export type PendingTask = VideoTaskStatus & {
  pending?: boolean;
};

/**
 * 视频页全局状态。提升到 App 层，避免在 tab 切换时（VideoPage 卸载）丢失：
 * - 已选课程 / 章节列表 / 进度
 * - 运行中任务（含乐观 pending 占位）
 * - 倍速、过滤、错误等 UI 偏好
 *
 * 同时这里集中订阅一次 video:// 事件，避免每次进入页面重订阅造成重复处理。
 */
export type VideoState = {
  courses: Course[];
  selected: Course | null;
  leaves: LeafNode[];
  schedule: Record<string, number>;
  tasks: PendingTask[];
  picked: Set<number>;
  speed: number;
  hideFinished: boolean;
  loading: boolean;
  submitting: boolean;
  error: string | null;
};

export type VideoActions = {
  setCourses: (c: Course[]) => void;
  setSelected: (c: Course | null) => void;
  setLeaves: (l: LeafNode[]) => void;
  setSchedule: (s: Record<string, number>) => void;
  setPicked: (s: Set<number>) => void;
  setSpeed: (n: number) => void;
  setHideFinished: (b: boolean) => void;
  setLoading: (b: boolean) => void;
  setSubmitting: (b: boolean) => void;
  setError: (s: string | null) => void;
  setTasks: (
    next: PendingTask[] | ((prev: PendingTask[]) => PendingTask[])
  ) => void;
};

export function useVideoState(): VideoState & VideoActions {
  const [courses, setCourses] = useState<Course[]>([]);
  const [selected, setSelected] = useState<Course | null>(null);
  const [leaves, setLeaves] = useState<LeafNode[]>([]);
  const [schedule, setSchedule] = useState<Record<string, number>>({});
  const [tasks, setTasks] = useState<PendingTask[]>([]);
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [speed, setSpeed] = useState(1);
  const [hideFinished, setHideFinished] = useState(false);
  const [loading, setLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 用 ref 持有最新的 selected，事件回调里实时读
  const selectedRef = useRef<Course | null>(null);
  useEffect(() => {
    selectedRef.current = selected;
  }, [selected]);

  const refreshSchedule = useCallback(async (c: Course) => {
    try {
      const s = await api.courseSchedule(c.classroom_id, c.sign);
      setSchedule(s);
    } catch {
      /* ignore */
    }
  }, []);

  // 首次挂载：拉一次后端已有任务 + 订阅事件（全局只订一次）+ 套用设置里的默认倍速
  useEffect(() => {
    api.listVideoTasks().then((arr) => setTasks(arr as PendingTask[]));
    // 套用设置中的默认倍速
    api.getSettings().then((s) => {
      if (s.video_speed && s.video_speed > 0) setSpeed(s.video_speed);
    });
    const unsubP = onVideoEvents({
      onProgress: (p) =>
        setTasks((arr) => {
          // 收到真实进度：移除该 leaf 的 pending 占位 + 替换/插入真实 task
          const filtered = arr.filter(
            (x) =>
              x.task_id !== p.task_id &&
              !(x.pending && x.leaf_id === p.leaf_id)
          );
          return [...filtered, p];
        }),
      onDone: (p) => {
        setTasks((arr) => arr.map((x) => (x.task_id === p.task_id ? { ...p } : x)));
        const cur = selectedRef.current;
        if (cur && cur.classroom_id === p.classroom_id) {
          refreshSchedule(cur);
        }
      },
      onError: (p) =>
        setTasks((arr) =>
          arr.map((x) =>
            x.task_id === p.task_id ? { ...x, error: p.message } : x
          )
        ),
    });
    // 订阅设置变更 → 实时同步倍速
    const unsubS = onSettingsUpdated((s) => {
      if (s.video_speed && s.video_speed > 0) setSpeed(s.video_speed);
    });
    return () => {
      unsubP.then((fn) => fn());
      unsubS.then((fn) => fn());
    };
  }, [refreshSchedule]);

  return {
    courses,
    selected,
    leaves,
    schedule,
    tasks,
    picked,
    speed,
    hideFinished,
    loading,
    submitting,
    error,
    setCourses,
    setSelected,
    setLeaves,
    setSchedule,
    setPicked,
    setSpeed,
    setHideFinished,
    setLoading,
    setSubmitting,
    setError,
    setTasks,
  };
}
