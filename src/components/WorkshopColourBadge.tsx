import { swatchColours } from './workshopList';

/** The round colour badge of a colour text, as the list draws it. Nothing when the text names no known colour. */
export default function WorkshopColourBadge({ text, className = '' }: { text: string; className?: string }) {
  const colours = swatchColours(text);
  if (colours.length === 0) return null;
  return <span className={`workshop-swatch ${className}`.trim()} aria-hidden="true">
    {colours.map((colour, index) => <i key={index} style={{ backgroundColor: colour }} />)}
  </span>;
}
