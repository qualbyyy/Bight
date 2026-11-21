import type { SVGProps } from "react";

type BightLogoProps = Omit<SVGProps<SVGSVGElement>, "fill"> & {
  size?: number;
};

export function BightLogo({ size = 18, ...props }: BightLogoProps) {
  return (
    <svg
      viewBox="0 0 100 100"
      width={size}
      height={size}
      fill="currentColor"
      focusable="false"
      aria-hidden="true"
      {...props}
    >
      <path d="M43.64 34.75C35.72 33.64 28 30.5 21 20.5C17.2 14.5 21.5 9.5 29.5 14.2L79.2 43.3C87.2 47.9 87.2 52.1 79.2 56.7L29.5 85.8C21.5 90.5 17.2 85.5 21 79.5C28 69.5 35.72 66.36 43.64 65.25A15.4 15.4 0 0 0 43.64 34.75Z" />
      <circle cx="41.5" cy="50" r="11" />
    </svg>
  );
}
