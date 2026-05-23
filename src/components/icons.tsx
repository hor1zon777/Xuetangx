import type { SVGProps } from "react";

export function WeChatIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
      {...props}
    >
      <path d="M9.4 3C5 3 1.5 5.95 1.5 9.6c0 2.07 1.13 3.91 2.9 5.13l-.62 1.9c-.07.21.15.39.34.27l2.27-1.39c.68.18 1.4.29 2.15.31a6.3 6.3 0 0 1-.16-1.4c0-3.3 3.13-5.97 7-5.97.17 0 .33 0 .49.02C15.4 5.18 12.66 3 9.4 3Zm-2.6 3.55a1 1 0 1 1 0 2 1 1 0 0 1 0-2Zm5.2 0a1 1 0 1 1 0 2 1 1 0 0 1 0-2ZM15.43 9.5C12.04 9.5 9.3 11.84 9.3 14.7c0 1.62.86 3.06 2.21 4.02l-.5 1.5c-.06.18.13.34.29.24l1.92-1.18c.66.19 1.36.3 2.1.32 3.46.07 6.43-2.16 6.5-5 .07-2.87-2.68-5.24-6.07-5.31a8.07 8.07 0 0 0-.32-.01Zm-1.97 2.85a.85.85 0 1 1 0 1.7.85.85 0 0 1 0-1.7Zm4.04 0a.85.85 0 1 1 0 1.7.85.85 0 0 1 0-1.7Z" />
    </svg>
  );
}

export function ShieldIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...props}
    >
      <path d="M12 3 4.5 6v6c0 4.6 3.2 7.9 7.5 9 4.3-1.1 7.5-4.4 7.5-9V6L12 3Z" />
      <path d="m9 12 2.2 2.2L15.5 10" />
    </svg>
  );
}

export function SparkIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
      {...props}
    >
      <path d="M12 2.5 13.6 8 19 9.6 13.6 11.2 12 16.6 10.4 11.2 5 9.6 10.4 8 12 2.5Zm6.5 11.4.7 2.4 2.4.7-2.4.7-.7 2.4-.7-2.4-2.4-.7 2.4-.7.7-2.4Z" />
    </svg>
  );
}

/**
 * 题库图标：合上的书 + 顶部书签。用于「📚 收录答案 / 题库命中」等场景。
 * stroke 跟随父元素 color（用 currentColor），所以可以放进 action-blue / ink-muted
 * 等各种文字色的按钮里直接继承颜色。
 */
export function BookIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...props}
    >
      <path d="M5 4.5A1.5 1.5 0 0 1 6.5 3h11A1.5 1.5 0 0 1 19 4.5v15a.5.5 0 0 1-.78.41L12 16l-6.22 3.91A.5.5 0 0 1 5 19.5v-15Z" />
      <path d="M9 8h6" />
      <path d="M9 11h4" />
    </svg>
  );
}

/**
 * 刷新图标：经典的循环箭头。用于「刷新进度」等可重复触发的按钮。
 */
export function RefreshIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...props}
    >
      <path d="M20 11A8 8 0 0 0 6.3 6.3L4 8.5" />
      <path d="M4 4v4.5h4.5" />
      <path d="M4 13a8 8 0 0 0 13.7 4.7L20 15.5" />
      <path d="M20 20v-4.5h-4.5" />
    </svg>
  );
}
