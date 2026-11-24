import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { errText } from '../api';
import { inventoryApi, type ItemSuggestion } from '../inventoryApi';
import { useToast } from './Toast';
import WorkshopColourBadge from './WorkshopColourBadge';
import {
  changeCount, columnTick, defaultTicks, filterRows, setColumnTicks, suggestionChanges, tickKey, ticksAfterReload,
  type SuggestionField, type SuggestionFilter,
} from './workshopIntake.ts';

interface Props {
  /** Reload the item list, the categories and the settings after items changed. Rejects when the list cannot be reloaded. */
  onChanged: () => Promise<void>;
  /** The number of suggestions after each load, for the label of the button that opened this screen. */
  onCount: (count: number) => void;
  onDone: () => void;
}

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;
const filters: ReadonlyArray<{ id: SuggestionFilter; label: string }> = [
  { id: 'all', label: 'All' }, { id: 'category', label: 'Category' }, { id: 'colour', label: 'Colour' },
];

/** A Select all/none box for one column. It is half-ticked when only some of the rows are. */
function ColumnBox({ label, state, onChange }: { label: string; state: 'all' | 'none' | 'some'; onChange: (on: boolean) => void }) {
  const box = useRef<HTMLInputElement>(null);
  useLayoutEffect(() => { if (box.current) box.current.indeterminate = state === 'some'; }, [state]);
  return <label className="workshop-suggest-all small">
    <input ref={box} type="checkbox" checked={state === 'all'} onChange={event => onChange(event.target.checked)} />
    <span>Select all/none<span className="sr-only"> {label}</span></span>
  </label>;
}

/**
 * Review suggestions: proposed categories and colours for items that already exist. Nothing changes until Apply, and
 * Apply sends exactly the ticked boxes of the rows in view.
 */
export default function WorkshopSuggestions({ onChanged, onCount, onDone }: Props) {
  const toast = useToast();
  const headingId = useId();
  const [rows, setRows] = useState<ItemSuggestion[] | null>(null);
  const [loadError, setLoadError] = useState('');
  const [ticked, setTicked] = useState<ReadonlySet<string>>(() => new Set());
  const [filter, setFilter] = useState<SuggestionFilter>('all');
  const [busy, setBusy] = useState(false);
  const [applyError, setApplyError] = useState('');
  const [status, setStatus] = useState('');
  const heading = useRef<HTMLHeadingElement>(null);
  const mounted = useRef(true);
  const rowsNow = useRef<ItemSuggestion[]>([]);
  const request = useRef(0);

  useEffect(() => {
    mounted.current = true;
    heading.current?.focus();
    return () => { mounted.current = false; };
  }, []);

  /** Loads the suggestions. The first load ticks the category changes; a later one keeps the designer's ticks. */
  async function load(first: boolean): Promise<boolean> {
    const mine = ++request.current;
    setLoadError('');
    try {
      const result = await inventoryApi.reviewSuggestions();
      if (!mounted.current || mine !== request.current) return true;
      setTicked(previous => first ? defaultTicks(result) : ticksAfterReload(rowsNow.current, result, previous));
      rowsNow.current = result;
      setRows(result);
      onCount(result.length);
      return true;
    } catch (reason) {
      if (mounted.current && mine === request.current) setLoadError(errText(reason));
      return false;
    }
  }

  useEffect(() => { void load(true); }, []);

  const shown = rows ? filterRows(rows, filter) : [];
  const changes = rows ? suggestionChanges(rows, ticked, filter) : [];
  const count = changeCount(changes);
  const showCategory = filter !== 'colour';
  const showColour = filter !== 'category';
  const categoryRows = rows ? filterRows(rows, 'category').length : 0;
  const colourRows = rows ? filterRows(rows, 'colour').length : 0;

  function toggle(field: SuggestionField, id: string, on: boolean) {
    setTicked(previous => {
      const next = new Set(previous);
      if (on) next.add(tickKey(field, id));
      else next.delete(tickKey(field, id));
      return next;
    });
  }

  async function apply() {
    if (busy || changes.length === 0) return;
    setBusy(true);
    setApplyError('');
    setStatus('');
    let changed: number;
    try {
      changed = await inventoryApi.applySuggestions(changes);
    } catch (reason) {
      if (mounted.current) { setApplyError(errText(reason)); setBusy(false); }
      return;
    }
    // The items did change. A reload that fails afterwards is reported, not mistaken for a failed apply.
    let reloadError = '';
    try { await onChanged(); } catch (reason) { reloadError = errText(reason); }
    toast.info(`${plural(changed, 'item', 'items')} updated`);
    if (!mounted.current) return;
    await load(false);
    if (!mounted.current) return;
    setBusy(false);
    setStatus(`${plural(changed, 'item', 'items')} updated.`);
    if (reloadError) setApplyError(`The list could not be reloaded. ${reloadError}`);
    heading.current?.focus();
  }

  return <div className="workshop-suggestions" aria-labelledby={headingId} role="region">
    <div className="workshop-heading">
      <h2 id={headingId} ref={heading} tabIndex={-1}>Review suggestions</h2>
      <button type="button" className="btn small" disabled={busy} onClick={onDone}>Done</button>
    </div>
    <p className="small muted">Proposed categories and colours for items you already have. Nothing changes until you apply it.</p>
    {loadError && <div className="panel">
      <p className="error-text" role="alert">Could not load the suggestions. {loadError}</p>
      <button type="button" className="btn small" onClick={() => void load(rows === null)}>Retry</button>
    </div>}
    {rows === null && !loadError && <p role="status">Loading suggestions…</p>}
    {rows !== null && rows.length === 0 && !loadError && <div className="panel workshop-suggest-empty">
      <p>No suggestions. Every item has a category and colour that match its name.</p>
      <button type="button" className="btn small" onClick={onDone}>Done</button>
    </div>}
    {rows !== null && rows.length > 0 && <>
      <div className="workshop-chips" role="group" aria-label="Show">
        {filters.map(chip => <button key={chip.id} type="button" className="workshop-chip" aria-pressed={filter === chip.id}
          onClick={() => setFilter(chip.id)}>
          {chip.label} <span className="workshop-chip-count">{chip.id === 'all' ? rows.length : chip.id === 'category' ? categoryRows : colourRows}</span>
        </button>)}
      </div>
      <div className="workshop-table workshop-suggest-table">
        <table>
          <caption className="sr-only">Suggested changes</caption>
          <thead>
            <tr>
              <th scope="col">Item</th>
              {showCategory && <th scope="col">
                <span className="workshop-suggest-head">Category
                  <ColumnBox label="categories" state={columnTick(shown, ticked, 'category')}
                    onChange={on => setTicked(previous => setColumnTicks(shown, previous, 'category', on))} /></span>
              </th>}
              {showColour && <th scope="col">
                <span className="workshop-suggest-head">Colour
                  <ColumnBox label="colours" state={columnTick(shown, ticked, 'colour')}
                    onChange={on => setTicked(previous => setColumnTicks(shown, previous, 'colour', on))} /></span>
              </th>}
            </tr>
          </thead>
          <tbody>
            {shown.map(row => <tr key={row.id}>
              <th scope="row" className="workshop-suggest-item">{row.name}</th>
              {showCategory && <td>
                {row.category_to !== null ? <label className="workshop-suggest-cell">
                  <input type="checkbox" checked={ticked.has(tickKey('category', row.id))}
                    aria-label={`Change the category of ${row.name} from ${row.category_from || 'none'} to ${row.category_to}`}
                    onChange={event => toggle('category', row.id, event.target.checked)} />
                  <span>
                    <span>{row.category_from || 'No category'} → <strong>{row.category_to}</strong></span>
                    {row.category_reason && <span className="small muted workshop-suggest-reason">{row.category_reason}</span>}
                  </span>
                </label> : <span className="workshop-unknown"><span aria-hidden="true">—</span><span className="sr-only">No category change</span></span>}
              </td>}
              {showColour && <td>
                {row.colour_to !== null ? <label className="workshop-suggest-cell">
                  <input type="checkbox" checked={ticked.has(tickKey('colour', row.id))}
                    aria-label={`Set the colour of ${row.name} to ${row.colour_to}`}
                    onChange={event => toggle('colour', row.id, event.target.checked)} />
                  <span>
                    <span className="workshop-colour"><WorkshopColourBadge text={row.colour_to} />Colour: <strong>{row.colour_to}</strong></span>
                    {row.colour_reason && <span className="small muted workshop-suggest-reason">{row.colour_reason}</span>}
                  </span>
                </label> : <span className="workshop-unknown"><span aria-hidden="true">—</span><span className="sr-only">No colour change</span></span>}
              </td>}
            </tr>)}
          </tbody>
        </table>
      </div>
      {applyError && <p className="error-text" role="alert">{applyError}</p>}
      <div className="workshop-actions workshop-suggest-actions">
        <button type="button" className="btn small" disabled={busy} onClick={onDone}>Cancel</button>
        <button type="button" className="btn primary" disabled={busy || count === 0} onClick={() => void apply()}>
          {busy ? 'Applying…' : `Apply ${plural(count, 'change', 'changes')}`}
        </button>
      </div>
    </>}
    {rows !== null && rows.length === 0 && applyError && <p className="error-text" role="alert">{applyError}</p>}
    <p className="sr-only" role="status" aria-live="polite">{status}</p>
  </div>;
}
