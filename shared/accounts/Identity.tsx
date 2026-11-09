"use client";
import { useState, type ReactNode } from "react";
// The calendar's rule (iniziali, web-ts lib/oe/logic.ts), so a company without a logo shows the same letters on both
// sites: "agap2" -> "AG", "Beta Servizi" -> "BS", "a.b@x" -> "AB"
export function initials(name: string): string {
  const s = name.split("·")[0].trim().split("@")[0];
  const words = s.split(/[\s._\-+]/).filter(Boolean);
  if (words.length >= 2) return (words[0][0] + words[1][0]).toUpperCase();
  return s.slice(0, 2).toUpperCase();
}
// the calendar's colour of an account with no calendar of its own (MicrosoftAccounts.tsx)
const NO_LOGO = "#64748b";
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
          <span
            className="grid size-full place-items-center text-[10px] leading-none font-extrabold text-white"
            style={{ background: NO_LOGO }}
          >
            {initials(name)}
          </span>
        )}
      </span>
      {children}
    </span>
  );
}
