import logoUrl from '../assets/logo.png';

/**
 * The Corydoras mark: the app icon (the catfish on its green tile), so the
 * window shows the same fish as the Dock. The source art is assets/icon-source.png;
 * src/assets/logo.png is a 256 px copy for the interface.
 */
export default function Logo({ size = 36 }: { size?: number }) {
  return <img className="logo" src={logoUrl} width={size} height={size} alt="" draggable={false} />;
}
