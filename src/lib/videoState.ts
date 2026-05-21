import React, { useCallback, useEffect, useRef, useState } from "react";
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
  setCourses: React.Dispatch<React.SetStateAction<Course[]>>;
  setSelected: React.Dispatch<React.SetStateAction<Course | null>>;
  setLeaves: React.Dispatch<React.SetStateAction<LeafNode[]>>;
  setSchedule: React.Dispatch<React.SetStateAction<Record<string, number>>>;
  setPicked: React.Dispatch<React.SetStateAction<Set<number>>>;
  setSpeed: React.Dispatch<React.SetStateAction<number>>;
  setHideFinished: React.Dispatch<React.SetStateAction<boolean>>;
  setLoading: React.Dispatch<React.SetStateAction<boolean>>;
  setSubmitting: React.Dispatch<React.SetStateAction<boolean>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setTasks: React.Dispatch<React.SetStateAction<PendingTask[]>>;
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
          // 收到真实进度：
          // - 若已有同 task_id 的真实任务 → 替换
          // - 否则插入新任务
          // 注意：不再"按 leaf_id 删 pending"，pending 应当由
          // startVideoTask 的返回值精确地用真实 task_id 替换。
          const i = arr.findIndex((x) => x.task_id === p.task_id);
          if (i >= 0) {
            const next = arr.slice();
            next[i] = p;
            return next;
          }
          return [...arr, p];
        }),
      onDone: (p) => {
        setTasks((arr) => arr.map((x) => (x.task_id === p.task_id ? { ...p } : x)));
        // 只有"正常播完"才标记为已完成。
        // cancelled=true（用户停止）或 error 非空（心跳失败终止）都不能把
        // schedule 写成 1，否则视频列表会展示假完成。
        if (!p.error && !p.cancelled) {
          setSchedule((s) => ({ ...s, [String(p.leaf_id)]: 1 }));
        }
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
