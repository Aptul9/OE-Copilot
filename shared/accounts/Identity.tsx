"use client";
import { useState, type ReactNode } from "react";
export const initials = (name: string) =>
  name
    .split(/[\s@._-]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0])
    .join("")
    .toUpperCase();
// The company logo in a white circle, as in the calendar sidebar; initials without one. The one picture of an account
// wherever it shows: Microsoft accounts in both apps, the account menu and the settings of TeamsRelay. children: a badge
// on its edge (the presence dot of TeamsRelay).
export function Logo({
  name,
  src,
  className = "size-7",
  faded,
  children,
}: {
  name: string;
  src?: string;
  className?: string;
  faded?: boolean;
  children?: ReactNode;
}) {
  const [broken, setBroken] = useState<string>();
  return (
    <span
      className={`relative inline-grid shrink-0 place-items-center rounded-full bg-white ring-1 ring-black/10 ${faded ? "opacity-50 grayscale" : ""} ${className}`}
    >
      <span aria-hidden="true" className="grid size-full place-items-center overflow-hidden rounded-full">
        {src && broken !== src ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={src}
            alt=""
            className="size-full object-contain p-[16%]"
            onError={() => setBroken(src)}
          />
        ) : (
          <span className="grid size-full place-items-center bg-primary text-[10px] font-bold text-primary-foreground">
            {initials(name)}
          </span>
        )}
      </span>
      {children}
    </span>
  );
}
