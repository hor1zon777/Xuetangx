import clsx from "clsx";
import type { ReactNode } from "react";

export function Pill({
  children,
  variant = "primary",
  onClick,
  disabled,
  type = "button",
  className,
}: {
  children: ReactNode;
  variant?: "primary" | "ghost";
  onClick?: () => void;
  disabled?: boolean;
  type?: "button" | "submit";
  className?: string;
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      className={clsx(variant === "primary" ? "pill-primary" : "pill-ghost", className)}
    >
      {children}
    </button>
  );
}

export function Capsule({
  children,
  onClick,
  selected,
  className,
}: {
  children: ReactNode;
  onClick?: () => void;
  selected?: boolean;
  className?: string;
}) {
  return (
    <button
      onClick={onClick}
      className={clsx(
        "pearl-capsule",
        selected && "ring-2 ring-action-blue-focus",
        className
      )}
    >
      {children}
    </button>
  );
}

export function Tile({
  children,
  variant = "light",
  className,
}: {
  children: ReactNode;
  variant?: "light" | "parchment" | "dark";
  className?: string;
}) {
  return (
    <section
      className={clsx(
        "w-full",
        variant === "light" && "tile-light",
        variant === "parchment" && "tile-parchment",
        variant === "dark" && "tile-dark",
        className
      )}
    >
      {children}
    </section>
  );
}

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={clsx("util-card", className)}>{children}</div>;
}

export function Field({
  label,
  children,
  hint,
}: {
  label: string;
  children: ReactNode;
  hint?: string;
}) {
  return (
    <label className="block">
      <div className="text-caption text-ink-muted-80 mb-1">{label}</div>
      {children}
      {hint && <div className="text-fine text-ink-muted-48 mt-1">{hint}</div>}
    </label>
  );
}

export function SectionTitle({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <header className="px-12 pt-12 pb-6">
      <h2 className="font-display text-display-md text-ink">{title}</h2>
      {subtitle && <p className="text-body text-ink-muted-80 mt-2">{subtitle}</p>}
    </header>
  );
}

export function Spinner() {
  return (
    <div className="inline-block w-4 h-4 rounded-full border-2 border-action-blue/30 border-t-action-blue animate-spin" />
  );
}
