import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, Columns3 } from 'lucide-react';
import { columnIds, type ColumnConfig, type ColumnId } from '../inventoryApi';
import { columnDefs } from './workshopColumns';

interface Props {
  /** "Filament" or "All categories". */
  scopeLabel: string;
  /** The stored choice for this view. null is Auto. */
  config: ColumnConfig;
  /** The columns that the view shows now: the Auto choice, or the stored choice. */
  shown: ColumnId[];
  /** Save a new choice. The parent updates the list at once and rolls back when the save fails. */
  onChange: (next: ColumnConfig) => void;
  /** The text of the button. The category manager uses "Columns…" with the category name as the accessible name. */
  buttonLabel?: string;
  /** Name for screen readers when several pickers are on one page, for example "Columns of Filament". */
  buttonName?: string;
  /** The panel opens towards the left when the button sits at the right edge. */
  align?: 'left' | 'right';
}

/** A "Columns" button that opens a panel to choose and order the columns of the current view. */
export default function WorkshopColumnPicker({ scopeLabel, config, shown, onChange, buttonLabel = 'Columns', buttonName, align = 'left' }: Props) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState('');
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const refocus = useRef<{ id: ColumnId; move: 'up' | 'down' } | null>(null);
  const panelId = useId();
  const auto = config === null;

  useEffect(() => {
    if (!open) return;
    const away = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', away);
    return () => document.removeEventListener('pointerdown', away);
  }, [open]);

  // A move can reorder the rows. Put focus back on the button that was pressed.
  useLayoutEffect(() => {
    const target = refocus.current;
    if (!target) return;
    refocus.current = null;
    root.current?.querySelector<HTMLButtonElement>(`button[data-column="${target.id}"][data-move="${target.move}"]`)?.focus();
  });

  const chosen = auto ? shown : shown.filter(id => config.includes(id));
  const rows: ColumnId[] = [...chosen, ...columnIds.filter(id => !chosen.includes(id))];

  function toggle(id: ColumnId, on: boolean) {
    if (auto) return;
    onChange(on ? [...chosen, id] : chosen.filter(other => other !== id));
    setNote(`${columnDefs[id].label} ${on ? 'shown' : 'hidden'}.`);
  }

  function move(id: ColumnId, by: -1 | 1) {
    const from = chosen.indexOf(id);
    const to = from + by;
    if (auto || from < 0 || to < 0 || to >= chosen.length) return;
    refocus.current = { id, move: by < 0 ? 'up' : 'down' };
    const next = [...chosen];
    next.splice(from, 1);
    next.splice(to, 0, id);
    onChange(next);
    setNote(`${columnDefs[id].label} moved to position ${to + 1} of ${next.length}.`);
  }

  function close() {
    setOpen(false);
    button.current?.focus();
  }

  return <div className={`workshop-picker${align === 'right' ? ' align-right' : ''}`} ref={root}
    onKeyDown={event => { if (open && event.key === 'Escape') { event.stopPropagation(); close(); } }}
    onBlur={event => {
      const next = event.relatedTarget as Node | null;
      if (open && next && !root.current?.contains(next)) setOpen(false);
    }}>
    <button type="button" className="btn small" ref={button} aria-expanded={open} aria-controls={panelId}
      aria-label={buttonName} onClick={() => setOpen(value => !value)}>
      <Columns3 size={14} aria-hidden="true" /> {buttonLabel}
    </button>
    {open && <div className="workshop-picker-panel" id={panelId} role="group" aria-labelledby={`${panelId}-title`}>
      <h2 className="workshop-picker-title" id={`${panelId}-title`}>Columns for {scopeLabel}</h2>
      <label className="workshop-picker-auto">
        <input type="checkbox" role="switch" checked={auto}
          onChange={event => onChange(event.target.checked ? null : [...shown])} />
        <span>Auto: hide empty columns</span>
      </label>
      {auto && <p className="workshop-picker-note">The columns that have a value are shown. Turn Auto off to choose them.</p>}
      <p className="sr-only" role="status">{note}</p>
      <ul className="workshop-picker-list" aria-label="Columns">
        <li className="workshop-picker-row fixed">
          <input type="checkbox" checked disabled aria-label="Item (always shown)" />
          <span className="workshop-picker-name">Item <span className="workshop-picker-hint">always shown</span></span>
        </li>
        {rows.map(id => {
          const index = chosen.indexOf(id);
          const on = index >= 0;
          const label = columnDefs[id].label;
          const first = !on || auto || index === 0;
          const last = !on || auto || index === chosen.length - 1;
          return <li key={id} className="workshop-picker-row">
            <input type="checkbox" checked={on} disabled={auto} aria-label={label}
              onChange={event => toggle(id, event.target.checked)} />
            <span className="workshop-picker-name">{label}</span>
            <button type="button" className="icon-btn" data-column={id} data-move="up"
              aria-label={`Move ${label} up`} aria-disabled={first || undefined}
              onClick={() => { if (!first) move(id, -1); }}>
              <ArrowUp size={14} aria-hidden="true" />
            </button>
            <button type="button" className="icon-btn" data-column={id} data-move="down"
              aria-label={`Move ${label} down`} aria-disabled={last || undefined}
              onClick={() => { if (!last) move(id, 1); }}>
              <ArrowDown size={14} aria-hidden="true" />
            </button>
          </li>;
        })}
      </ul>
      <div className="workshop-picker-foot">
        <button type="button" className="btn small" aria-disabled={auto || undefined}
          onClick={() => { if (!auto) { onChange(null); setNote('Columns reset to Auto.'); } }}>Reset</button>
        <button type="button" className="btn small" onClick={close}>Done</button>
      </div>
    </div>}
  </div>;
}
