/**
 * The Corydoras mark: the catfish on a rounded tile, as in the app icon.
 * The fish takes the text colour and the eye takes the accent, so the mark works in light and dark mode.
 */
export default function Logo({ size = 36, tile = true }: { size?: number; tile?: boolean }) {
  const ring = tile ? 'var(--logo-tile)' : 'var(--logo-ring, var(--bg))';
  return (
    <svg
      className={`logo ${tile ? 'logo-tile' : ''}`}
      width={size}
      height={tile ? size : size * 0.64}
      viewBox={tile ? '96 96 832 832' : '120 225 785 500'}
      aria-hidden="true"
    >
      {tile && <rect x="100" y="100" width="824" height="824" rx="190" fill="var(--logo-tile)" stroke="var(--rule)" strokeWidth="8" />}
      <path
        fill="currentColor"
        d="M175 525 C175 440 300 400 400 398 L482 238 L540 418 C620 440 700 470 752 500 L890 408 L852 525 L890 632 L748 548 C680 580 560 615 468 625 L442 717 L382 628 C280 625 175 600 175 525 Z"
      />
      <g stroke="currentColor" strokeWidth="14">
        <line x1="172" y1="540" x2="134" y2="598" />
        <line x1="190" y1="566" x2="162" y2="622" />
      </g>
      <line x1="343" y1="535" x2="712" y2="521" stroke={ring} strokeWidth="12" />
      <circle cx="282" cy="488" r="43" fill={ring} />
      <circle cx="282" cy="488" r="31" fill="var(--accent)" />
    </svg>
  );
}
