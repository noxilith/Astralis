import { useId } from 'react'

/** The Astralis mark: a peak with an inner arch and a four-point star. Colors follow the theme. */
export function AstralisMark({ size = 26 }: { size?: number }) {
  const id = useId().replace(/:/g, '')
  return (
    <svg width={size} height={size * 0.78} viewBox="200 270 854 620" aria-hidden="true">
      <defs>
        <linearGradient id={`${id}-fill`} x1="0" y1="287" x2="0" y2="872" gradientUnits="userSpaceOnUse">
          <stop offset="0" style={{ stopColor: 'var(--logo-a)' }} />
          <stop offset="1" style={{ stopColor: 'var(--logo-b)' }} />
        </linearGradient>
        <filter id={`${id}-glow`} x="-50%" y="-50%" width="200%" height="200%">
          <feGaussianBlur stdDeviation="16" />
        </filter>
      </defs>
      <g fill={`url(#${id}-fill)`}>
        <path d="M627 287 L856 641 C770 630 712 594 685 552 L627 463 L569 552 C542 594 484 630 398 641 Z" />
        <path id={`${id}-wing`} d="M385 663 L445 663 C520 663 575 720 607 790 C580 765 555 745 515 745 C470 745 445 770 410 795 C360 830 290 855 222 872 C262 845 290 810 300 795 Z" />
        <use href={`#${id}-wing`} transform="translate(1254 0) scale(-1 1)" />
      </g>
      <path
        d="M627 545 C634 612 646 646 729 653 C646 660 634 694 627 765 C620 694 608 660 525 653 C608 646 620 612 627 545 Z"
        style={{ fill: 'var(--logo-glow)' }}
        filter={`url(#${id}-glow)`}
      />
      <path
        d="M627 545 C634 612 646 646 729 653 C646 660 634 694 627 765 C620 694 608 660 525 653 C608 646 620 612 627 545 Z"
        style={{ fill: 'var(--logo-star)' }}
      />
    </svg>
  )
}
