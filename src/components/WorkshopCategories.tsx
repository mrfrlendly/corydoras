import { useEffect, useId, useLayoutEffect, useRef, useState, type FormEvent } from 'react';
import { errText } from '../api';
import { inventoryApi, type ColumnConfig, type InventoryItem } from '../inventoryApi';
import { useToast } from './Toast';
import WorkshopColumnPicker from './WorkshopColumnPicker';
import {
  checkCurrency, findCategory, mergeTarget, mergeText, movedText, newCategoryError, renameError,
} from './workshopActions';
import { visibleColumns } from './workshopColumns';
import { matchingItems } from './workshopForm';

export interface CategoryRow {
  name: string;
  /** Items that use the category now. */
  count: number;
  columns: ColumnConfig;
}

interface Props {
  rows: CategoryRow[];
  /** All items: the Columns panel of a category needs them to resolve Auto. */
  items: InventoryItem[];
  /** Three-letter currency code of the library. */
  currency: string;
  /** Save the columns of one category. The list updates at once and rolls back when the save fails. */
  onColumns: (name: string, next: ColumnConfig) => void;
  /** Reload the item list, the categories and the settings after a change. Rejects when the list cannot be reloaded. */
  onChanged: () => Promise<void>;
  /** A category was renamed or merged: `to` is the name the items have now. */
  onRenamed: (from: string, to: string) => void;
  onDone: () => void;
}

type Edit = { kind: 'rename'; name: string; text: string } | { kind: 'merge'; name: string; into: string };

const deleteReason = 'Move or merge its items first.';
const plural = (count: number) => `${count} ${count === 1 ? 'item' : 'items'}`;

/** Create, rename, merge and delete categories, choose the columns of each, and set the currency of prices. */
export default function WorkshopCategories({ rows, items, currency, onColumns, onChanged, onRenamed, onDone }: Props) {
  const toast = useToast();
  const reasonId = useId();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [newName, setNewName] = useState('');
  const [edit, setEdit] = useState<Edit | null>(null);
  const [currencyText, setCurrencyText] = useState(currency);
  const [currencyError, setCurrencyError] = useState('');
  const root = useRef<HTMLDivElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const newInput = useRef<HTMLInputElement>(null);
  const currencyInput = useRef<HTMLInputElement>(null);
  // The currency save that is running, so that Done can wait for it instead of starting a second one.
  const currencySave = useRef<Promise<boolean> | null>(null);
  const mounted = useRef(true);
  // Where focus goes after a change has reloaded the rows: a row's button by category name, or the new-category field.
  const refocus = useRef<{ name: string; action: string } | 'new' | null>(null);

  useEffect(() => {
    mounted.current = true;
    heading.current?.focus();
    return () => { mounted.current = false; };
  }, []);

  useLayoutEffect(() => {
    const target = refocus.current;
    if (!target || busy) return;
    if (target === 'new') {
      refocus.current = null;
      newInput.current?.focus();
      return;
    }
    const found = findCategory(rows, target.name);
    if (!found) return;
    refocus.current = null;
    focusAction(found.name, target.action);
  });

  function focusAction(name: string, action: string) {
    root.current?.querySelector<HTMLElement>(`[data-category="${CSS.escape(name)}"][data-action="${action}"]`)?.focus();
  }

  /** Run one change, then reload. One change at a time. Errors stay on screen. */
  async function run(change: () => Promise<string | void>, after: typeof refocus.current = null) {
    if (busy) return false;
    setBusy(true);
    setError('');
    try {
      const message = await change();
      refocus.current = after;
      await onChanged();
      if (mounted.current && message) toast.info(message);
      return true;
    } catch (reason) {
      refocus.current = null;
      if (mounted.current) setError(errText(reason));
      return false;
    } finally {
      if (mounted.current) setBusy(false);
    }
  }

  async function add(event: FormEvent) {
    event.preventDefault();
    const problem = newCategoryError(rows, newName);
    if (problem) { setError(problem); return; }
    const name = newName.trim();
    const ok = await run(async () => { await inventoryApi.createCategory(name); return `Category ${name} added.`; }, 'new');
    if (ok && mounted.current) setNewName('');
  }

  async function rename(from: string, text: string) {
    const problem = renameError(from, text);
    if (problem) { setError(problem); return; }
    const to = text.trim();
    const target = mergeTarget(rows, from, to);
    if (target) {
      // A name that exists is a merge. It asks first, because the items move.
      setError('');
      setEdit({ kind: 'merge', name: from, into: target.name });
      return;
    }
    const ok = await run(async () => {
      const moved = await inventoryApi.renameCategory(from, to);
      onRenamed(from, to);
      return movedText(moved, to);
    }, { name: to, action: 'rename' });
    if (ok && mounted.current) setEdit(null);
  }

  async function merge(from: string, into: string) {
    const ok = await run(async () => {
      const moved = await inventoryApi.renameCategory(from, into);
      onRenamed(from, into);
      return movedText(moved, into);
    }, 'new');
    if (ok && mounted.current) setEdit(null);
  }

  async function remove(name: string) {
    await run(async () => { await inventoryApi.deleteCategory(name); return `Category ${name} deleted.`; }, 'new');
  }

  /** Saves the currency field. Resolves true when the library has the typed code (or it is unchanged), false with the error on screen. */
  function saveCurrency(): Promise<boolean> {
    if (!currencySave.current) {
      const job = storeCurrency().finally(() => { currencySave.current = null; });
      currencySave.current = job;
    }
    return currencySave.current;
  }

  async function storeCurrency(): Promise<boolean> {
    if (busy) return false;
    const result = checkCurrency(currencyText);
    if (!result.ok) { setCurrencyError(result.error); return false; }
    setCurrencyError('');
    setCurrencyText(result.code);
    if (result.code === currency) return true;
    setBusy(true);
    try {
      await inventoryApi.setCurrency(result.code);
      await onChanged();
      if (mounted.current) toast.info(`Prices are now in ${result.code}.`);
      return true;
    } catch (reason) {
      if (mounted.current) setCurrencyError(errText(reason));
      return false;
    } finally {
      if (mounted.current) setBusy(false);
    }
  }

  // Done never drops a currency that is typed but not saved: it saves it, or stays open with the error shown.
  async function finish() {
    if (currencyError || currencyText.trim().toUpperCase() !== currency) {
      const ok = await saveCurrency();
      if (!ok) {
        if (mounted.current) currencyInput.current?.focus();
        return;
      }
    }
    if (mounted.current) onDone();
  }

  function categoryColumns(row: CategoryRow) {
    return visibleColumns(matchingItems(items, '', row.name), row.columns, 'category');
  }

  return <div className="workshop-categories" ref={root}>
    <div className="workshop-heading">
      <h2 ref={heading} tabIndex={-1}>Categories</h2>
      <button type="button" className="btn small" onClick={() => void finish()}>Done</button>
    </div>
    {error && <p className="error-text" role="alert">{error}</p>}
    <section className="panel">
      <form className="workshop-cat-new" onSubmit={add}>
        <label className="field"><span>New category</span>
          <input ref={newInput} type="text" maxLength={120} placeholder="e.g. Bearings" value={newName} readOnly={busy}
            autoCapitalize="off" autoCorrect="off" spellCheck={false}
            onChange={event => { setNewName(event.target.value); setError(''); }} />
        </label>
        <button type="submit" className="btn small" disabled={busy}>Add</button>
      </form>
      <ul className="workshop-cat-list" aria-label="Categories">
        {rows.map((row, index) => {
          const editing = edit?.name === row.name ? edit : null;
          const used = row.count > 0;
          const others = rows.filter(other => other.name !== row.name);
          return <li key={row.name} className="workshop-cat-row" data-row={row.name}>
            <div className="workshop-cat-line">
              <span className="workshop-cat-name">{row.name}</span>
              <span className="workshop-cat-count small muted">{plural(row.count)}</span>
              <span className="workshop-cat-actions">
                <WorkshopColumnPicker scopeLabel={row.name} config={row.columns} shown={categoryColumns(row)}
                  onChange={next => onColumns(row.name, next)} buttonLabel="Columns…"
                  buttonName={`Columns of ${row.name}`} align="right" />
                <button type="button" className="btn small" data-category={row.name} data-action="rename"
                  aria-label={`Rename ${row.name}`} disabled={busy}
                  onClick={() => { setError(''); setEdit({ kind: 'rename', name: row.name, text: row.name }); }}>Rename</button>
                <button type="button" className="btn small" data-category={row.name} data-action="merge"
                  aria-label={`Merge ${row.name} into another category`} disabled={busy || others.length === 0}
                  onClick={() => { setError(''); setEdit({ kind: 'merge', name: row.name, into: '' }); }}>Merge into…</button>
                <button type="button" className="btn small ghost-danger" data-category={row.name} data-action="delete"
                  aria-label={`Delete ${row.name}`} aria-disabled={used || busy || undefined}
                  aria-describedby={used ? `${reasonId}-${index}` : undefined}
                  title={used ? deleteReason : undefined}
                  onClick={() => { if (!used && !busy) void remove(row.name); }}>Delete</button>
                {used && <span className="sr-only" id={`${reasonId}-${index}`}>{deleteReason}</span>}
              </span>
            </div>
            {editing?.kind === 'rename' && <form className="workshop-cat-edit" onSubmit={event => {
              event.preventDefault();
              void rename(row.name, editing.text);
            }}>
              <label className="field"><span>New name for {row.name}</span>
                <input type="text" autoFocus maxLength={120} value={editing.text} readOnly={busy}
                  autoCapitalize="off" autoCorrect="off" spellCheck={false}
                  onChange={event => { setEdit({ ...editing, text: event.target.value }); setError(''); }}
                  onKeyDown={event => {
                    if (event.key !== 'Escape') return;
                    event.preventDefault();
                    event.stopPropagation();
                    setEdit(null);
                    setError('');
                    focusAction(row.name, 'rename');
                  }} />
              </label>
              <button type="submit" className="btn small primary" disabled={busy}>Save name</button>
              <button type="button" className="btn small" disabled={busy}
                onClick={() => {
                  setEdit(null);
                  setError('');
                  focusAction(row.name, 'rename');
                }}>Cancel</button>
            </form>}
            {editing?.kind === 'merge' && <div className="workshop-cat-edit">
              <label className="field"><span>Merge {row.name} into</span>
                <select autoFocus value={editing.into} disabled={busy}
                  onChange={event => setEdit({ ...editing, into: event.target.value })}>
                  <option value="">Choose a category</option>
                  {others.map(other => <option key={other.name} value={other.name}>{other.name}</option>)}
                </select>
              </label>
              {editing.into && <p className="workshop-cat-confirm small" role="status">
                {mergeText(row.name, row.count, editing.into)}
              </p>}
              <button type="button" className="btn small primary" disabled={busy || !editing.into}
                onClick={() => void merge(row.name, editing.into)}>Confirm merge</button>
              <button type="button" className="btn small" disabled={busy}
                onClick={() => {
                  setEdit(null);
                  focusAction(row.name, 'merge');
                }}>Cancel</button>
            </div>}
          </li>;
        })}
      </ul>
      {rows.length === 0 && <p className="small muted">No categories yet.</p>}
    </section>
    <section className="panel workshop-currency" aria-label="Currency">
      <label className="field"><span>Prices are in</span>
        <input ref={currencyInput} type="text" maxLength={3} size={4} value={currencyText} readOnly={busy}
          autoCapitalize="characters" autoCorrect="off" spellCheck={false}
          aria-invalid={currencyError ? true : undefined}
          aria-describedby={currencyError ? 'workshop-currency-help workshop-currency-error' : 'workshop-currency-help'}
          onChange={event => { setCurrencyText(event.target.value); setCurrencyError(''); }}
          onBlur={() => void saveCurrency()}
          onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); void saveCurrency(); } }} />
      </label>
      <p className="small muted" id="workshop-currency-help">
        A three-letter code, such as CAD or USD. Changing it does not convert the prices you entered.
      </p>
      {currencyError && <p className="small error-text" id="workshop-currency-error" role="alert">{currencyError}</p>}
    </section>
  </div>;
}
