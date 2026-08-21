import type { SVGProps } from "react";

export type IconName =
  | "inbox"
  | "today"
  | "projects"
  | "history"
  | "timer"
  | "worklog"
  | "note"
  | "search"
  | "trash"
  | "database"
  | "plus"
  | "arrow"
  | "check"
  | "close"
  | "play"
  | "stop"
  | "calendar"
  | "flag"
  | "tag"
  | "link"
  | "chevron"
  | "spark"
  | "menu"
  | "restore"
  | "download"
  | "shield"
  | "clock"
  | "chart"
  | "mail"
  | "more";

interface IconProps extends SVGProps<SVGSVGElement> {
  name: IconName;
  size?: number;
}

export function Icon({ name, size = 18, ...props }: IconProps) {
  const body = (() => {
    switch (name) {
      case "inbox":
        return <><path d="M4 5.5h16l1.5 10.5H16l-2 2h-4l-2-2H2.5L4 5.5Z"/><path d="M4 12h4l2 2h4l2-2h4"/></>;
      case "today":
        return <><rect x="3" y="5" width="18" height="16" rx="2"/><path d="M8 3v4m8-4v4M3 10h18"/><path d="m9 15 2 2 4-5"/></>;
      case "projects":
        return <><path d="M3 7.5V5a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v10a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V7.5Z"/><path d="M3 9h18"/></>;
      case "history":
        return <><path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5m4-1v5l3 2"/></>;
      case "timer":
        return <><circle cx="12" cy="13" r="8"/><path d="M12 9v4l3 2M9 2h6m-3 3V2"/></>;
      case "worklog":
        return <><path d="M5 3h11l3 3v15H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z"/><path d="M15 3v4h4M7 11h8M7 15h8M7 19h5"/></>;
      case "note":
        return <><path d="M5 3h14a2 2 0 0 1 2 2v12l-4 4H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z"/><path d="M17 21v-4h4M7 8h10M7 12h7"/></>;
      case "search":
        return <><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></>;
      case "trash":
        return <><path d="M4 7h16M9 3h6l1 4H8l1-4Zm-3 4 1 14h10l1-14M10 11v6m4-6v6"/></>;
      case "database":
        return <><ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v7c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12v7c0 1.7 3.6 3 8 3s8-1.3 8-3v-7"/></>;
      case "plus":
        return <path d="M12 5v14M5 12h14"/>;
      case "arrow":
        return <path d="m5 12 6-6m-6 6 6 6m-6-6h14"/>;
      case "check":
        return <path d="m5 12 4 4L19 6"/>;
      case "close":
        return <path d="m6 6 12 12M18 6 6 18"/>;
      case "play":
        return <path d="m9 6 9 6-9 6V6Z"/>;
      case "stop":
        return <rect x="7" y="7" width="10" height="10" rx="1"/>;
      case "calendar":
        return <><rect x="3" y="5" width="18" height="16" rx="2"/><path d="M8 3v4m8-4v4M3 10h18"/></>;
      case "flag":
        return <><path d="M5 21V4m0 1h10l-1.5 3L15 11H5"/></>;
      case "tag":
        return <><path d="M3 11V4h7l11 11-6 6L3 11Z"/><circle cx="7.5" cy="7.5" r="1"/></>;
      case "link":
        return <><path d="m10 13 4-4"/><path d="M7.5 16.5 5 19a3.5 3.5 0 0 1-5-5l4-4a3.5 3.5 0 0 1 5 0M16.5 7.5 19 5a3.5 3.5 0 0 1 5 5l-4 4a3.5 3.5 0 0 1-5 0"/></>;
      case "chevron":
        return <path d="m9 18 6-6-6-6"/>;
      case "spark":
        return <><path d="m12 2 1.5 5.5L19 9l-5.5 1.5L12 16l-1.5-5.5L5 9l5.5-1.5L12 2Z"/><path d="m19 16 .7 2.3L22 19l-2.3.7L19 22l-.7-2.3L16 19l2.3-.7L19 16Z"/></>;
      case "menu":
        return <path d="M4 7h16M4 12h16M4 17h16"/>;
      case "restore":
        return <><path d="M4 4v6h6"/><path d="M5.5 16a8 8 0 1 0 .2-8L4 10"/></>;
      case "download":
        return <><path d="M12 3v12m-5-5 5 5 5-5M4 20h16"/></>;
      case "shield":
        return <><path d="M12 2 4 5v6c0 5 3.4 9 8 11 4.6-2 8-6 8-11V5l-8-3Z"/><path d="m8 12 2.5 2.5L16 9"/></>;
      case "clock":
        return <><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></>;
      case "chart":
        return <><path d="M4 20V10m5 10V4m5 16v-7m5 7V7"/><path d="m3 8 5-4 6 7 6-6"/></>;
      case "mail":
        return <><rect x="3" y="5" width="18" height="14" rx="2"/><path d="m4 7 8 6 8-6"/></>;
      case "more":
        return <><circle cx="5" cy="12" r="1" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1" fill="currentColor" stroke="none"/><circle cx="19" cy="12" r="1" fill="currentColor" stroke="none"/></>;
    }
  })();

  return (
    <svg
      aria-hidden="true"
      fill="none"
      height={size}
      viewBox="0 0 24 24"
      width={size}
      {...props}
    >
      <g stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.7">
        {body}
      </g>
    </svg>
  );
}
