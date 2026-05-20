import type { ProblemKind } from "../lib/api";

const META: Record<
  ProblemKind,
  { label: string; bg: string; fg: string }
> = {
  single_choice: {
    label: "单选",
    bg: "bg-action-blue/10",
    fg: "text-action-blue",
  },
  multiple_choice: {
    label: "多选",
    bg: "bg-[#8e44ad]/10",
    fg: "text-[#8e44ad]",
  },
  judgement: {
    label: "判断",
    bg: "bg-[#1abc9c]/10",
    fg: "text-[#159e85]",
  },
  completion: {
    label: "填空",
    bg: "bg-[#e67e22]/10",
    fg: "text-[#c25a09]",
  },
  subjective: {
    label: "主观",
    bg: "bg-[#d35400]/10",
    fg: "text-[#b3460a]",
  },
  other: {
    label: "其它",
    bg: "bg-parchment",
    fg: "text-ink-muted-80",
  },
};

export function KindBadge({
  kind,
  text,
}: {
  kind?: ProblemKind | string | null;
  text?: string;
}) {
  const m = (kind && (META as any)[kind]) ?? META.other;
  return (
    <span
      className={`inline-flex items-center text-fine leading-none h-[20px] px-2 ${m.bg} ${m.fg} rounded-pill whitespace-nowrap`}
    >
      {text ?? m.label}
    </span>
  );
}

export function kindLabel(kind?: ProblemKind | string | null): string {
  return ((kind && (META as any)[kind]) ?? META.other).label;
}
