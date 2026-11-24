import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Boxes, Camera, Plus, Search, Tags } from 'lucide-react';
import { errText } from '../api';
import {
  inventoryApi, type CategoryInfo, type ColumnConfig, type InventoryDraft, type InventoryItem, type ViewSettings,
} from '../inventoryApi';
import { useToast } from './Toast';
import WorkshopCategories from './WorkshopCategories';
import WorkshopColumnPicker from './WorkshopColumnPicker';
import WorkshopItemEditor from './WorkshopItemEditor';
import WorkshopPhotoReview from './WorkshopPhotoReview';
import { categoryChips, shortcutFor, stepQuantityItem } from './workshopActions';
import { categoryKey, duplicateForm, matchingItems, type WorkshopForm } from './workshopForm';
import WorkshopInventoryTable from './WorkshopInventoryTable';
import { viewTotals, visibleColumns, formatMoney } from './workshopColumns';
import { defaultListSort, effectiveSort, nextListSort, parseListSort, sortItems, type ListSort } from './workshopList';
import './workshop.css';

const preference = 'corydoras.workshop.inventory.sort';
const categoryPreference = 'corydoras.workshop.inventory.category';
const defaultView: ViewSettings = { currency: 'CAD', all_columns: null };

function readSort(): ListSort {
  try { return parseListSort(localStorage.getItem(preference)); }
  catch { return defaultListSort; }
}

function readCategory(): string {
  try { return localStorage.getItem(categoryPreference) ?? ''; }
  catch { return ''; }
}

/** The key under which a view's column choice is confirmed or pending. Category names are one name in any letter case. */
const viewKey = (name: string) => name ? `c:${categoryKey(name)}` : 'all';
const sameName = (a: string, b: string) => categoryKey(a) === categoryKey(b);

/** `tick` changes when the library changes elsewhere (for example a save from the phone): the list reloads, an open editor stays. */
export default function WorkshopInventory({ tick = 0 }: { tick?: number }) {
  const toast = useToast();
  const [items, setItems] = useState<InventoryItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const [editor, setEditor] = useState<InventoryItem | 'new' | null>(null);
  // A copy of an item (Duplicate item): the editor opens as a new item with these values.
  const [copy, setCopy] = useState<{ form: WorkshopForm; of: string; seed: number } | null>(null);
  const [manage, setManage] = useState(false);
  const [stepping, setStepping] = useState<ReadonlySet<string>>(() => new Set());
  const [stepNote, setStepNote] = useState('');
  const [review, setReview] = useState<{ file: File; drafts: InventoryDraft[] } | 'new' | null>(null);
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState(readCategory);
  const [noPrice, setNoPrice] = useState(false);
  const [noCount, setNoCount] = useState(false);
  const [sort, setSort] = useState(readSort);
  // Stored per-view settings. When a load fails these stay empty and every view is Auto in the library currency.
  const [categoryInfos, setCategoryInfos] = useState<CategoryInfo[]>([]);
  const [viewSettings, setViewSettings] = useState<ViewSettings>(defaultView);
  const addButton = useRef<HTMLButtonElement>(null);
  const manageButton = useRef<HTMLButtonElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const stepLock = useRef(new Set<string>());
  const returnTo = useRef<string | null>(null);
  const list = useRef<HTMLTableElement>(null);
  const focusAfterClose = useRef<string | null | undefined>(undefined);

  // The saved column choices and categories load beside the list and never block it: a failure leaves Auto.
  const confirmed = useRef(new Map<string, ColumnConfig>());
  // Column saves still in flight per view. A reload that arrives meanwhile must not overwrite their optimistic value.
  const pending = useRef(new Map<string, number>());
  const [categoriesReady, setCategoriesReady] = useState(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  function loadViewData(isCurrent: () => boolean = () => mounted.current): Promise<unknown> {
    const saving = (key: string) => (pending.current.get(key) ?? 0) > 0;
    const categoriesLoaded = inventoryApi.categories().then(result => {
      if (!isCurrent()) return;
      for (const info of result) if (!saving(viewKey(info.name))) confirmed.current.set(viewKey(info.name), info.columns);
      setCategoryInfos(previous => [
        ...result.map(info => {
          const held = saving(viewKey(info.name)) ? previous.find(other => sameName(other.name, info.name)) : undefined;
          return held ? { ...info, columns: held.columns } : info;
        }),
        // A category whose first column save has not reached the library yet is not in the answer.
        ...previous.filter(other => saving(viewKey(other.name)) && !result.some(info => sameName(info.name, other.name))),
      ]);
      setCategoriesReady(true);
    }).catch(() => { if (isCurrent()) setCategoriesReady(true); /* chips fall back to counts from the items */ });
    const settingsLoaded = inventoryApi.viewSettings().then(result => {
      if (!isCurrent()) return;
      if (!saving('all')) confirmed.current.set('all', result.all_columns);
      setViewSettings(previous => saving('all') ? { ...result, all_columns: previous.all_columns } : result);
    }).catch(() => { /* Auto, in the default currency */ });
    return Promise.all([categoriesLoaded, settingsLoaded]);
  }

  /** The list, the categories and the settings again, after a change in the category manager. Rejects when the list fails. */
  async function reloadAll() {
    await Promise.all([
      inventoryApi.list().then(result => { if (mounted.current) setItems(result); }),
      loadViewData(),
    ]);
  }

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError('');
    inventoryApi.list()
      .then(result => { if (active) setItems(result); })
      .catch(reason => { if (active) setError(errText(reason)); })
      .finally(() => { if (active) setLoading(false); });
    loadViewData(() => active);
    return () => { active = false; };
  }, [retry]);

  // A quiet reload: no loading state, and the editor or review stays as it is.
  const firstTick = useRef(tick);
  useEffect(() => {
    if (tick === firstTick.current) return;
    let active = true;
    inventoryApi.list()
      .then(result => { if (active) setItems(result); })
      .catch(() => { /* the list on screen stays; Retry is there if the next load fails */ });
    loadViewData(() => active);
    return () => { active = false; };
  }, [tick]);

  // Categories: the stored ones (including empty ones) and any that items use. Counts come from the items on screen.
  const chips = useMemo(() => categoryChips(items, categoryInfos), [categoryInfos, items]);
  const categories = useMemo(() => chips.map(chip => chip.name), [chips]);
  // The chip name that `category` stands for, whatever letter case it was saved in.
  const effectiveCategory = chips.find(chip => sameName(chip.name, category))?.name ?? '';
  const flags = useMemo(() => ({ noPrice, noCount }), [noPrice, noCount]);

  const viewItems = useMemo(() => matchingItems(items, '', effectiveCategory), [items, effectiveCategory]);
  const columnScope = effectiveCategory ? 'category' : 'all';
  const columnConfig: ColumnConfig = effectiveCategory
    ? categoryInfos.find(info => sameName(info.name, effectiveCategory))?.columns ?? null
    : viewSettings.all_columns;
  const columns = useMemo(() => visibleColumns(viewItems, columnConfig, columnScope),
    [viewItems, columnConfig, columnScope]);
  const shownSort = effectiveSort(sort, columns);
  const visible = useMemo(() => sortItems(matchingItems(items, query, effectiveCategory, flags), shownSort),
    [items, query, effectiveCategory, flags, shownSort]);
  const totals = useMemo(() => viewTotals(visible), [visible]);
  const currency = viewSettings.currency;

  function chooseCategory(name: string) {
    setCategory(name);
    try {
      if (name) localStorage.setItem(categoryPreference, name);
      else localStorage.removeItem(categoryPreference);
    } catch { /* The choice still holds for this session. */ }
  }

  // A remembered category that the library no longer has is forgotten, once the categories have loaded.
  // Not while the manager is open: a rename there names the category before the reload brings it.
  useEffect(() => {
    if (loading || error || !categoriesReady || manage || !category || effectiveCategory) return;
    chooseCategory('');
  }, [loading, error, categoriesReady, manage, category, effectiveCategory]);

  function applyColumns(name: string, next: ColumnConfig) {
    if (name) {
      setCategoryInfos(previous => previous.some(info => sameName(info.name, name))
        ? previous.map(info => sameName(info.name, name) ? { ...info, columns: next } : info)
        : [...previous, { name, count: 0, columns: next }]);
    } else setViewSettings(previous => ({ ...previous, all_columns: next }));
  }

  // Changes save at once, one after another. A failed save puts back the last choice that the library confirmed.
  const saves = useRef<Promise<void>>(Promise.resolve());
  const latestSave = useRef(new Map<string, number>());
  function changeColumns(next: ColumnConfig, name: string = effectiveCategory) {
    const key = viewKey(name);
    if (!confirmed.current.has(key)) {
      confirmed.current.set(key, name ? categoryInfos.find(info => sameName(info.name, name))?.columns ?? null : viewSettings.all_columns);
    }
    pending.current.set(key, (pending.current.get(key) ?? 0) + 1);
    const token = (latestSave.current.get(key) ?? 0) + 1;
    latestSave.current.set(key, token);
    applyColumns(name, next);
    saves.current = saves.current.then(async () => {
      try {
        await (name ? inventoryApi.setCategoryColumns(name, next) : inventoryApi.setAllColumns(next));
        confirmed.current.set(key, next);
      } catch (reason) {
        if (latestSave.current.get(key) === token && mounted.current) {
          applyColumns(name, confirmed.current.get(key) ?? null);
          toast.error(`Could not save the columns. ${errText(reason)}`);
        }
      } finally {
        pending.current.set(key, (pending.current.get(key) ?? 1) - 1);
      }
    });
  }
  // - and + on On hand. One request per item at a time. The count changes at once and goes back when the save fails.
  // Both responses are the whole item, so the open pack edit below shares this lock.
  async function stepItem(item: InventoryItem, delta: number) {
    if (stepLock.current.has(item.id)) return;
    const before = item.quantity ?? null;
    const left = item.measure_left ?? null;
    const stepped = stepQuantityItem(item, delta);
    if (stepped === null) return;
    const next = stepped.quantity as number;
    stepLock.current.add(item.id);
    setStepping(new Set(stepLock.current));
    // A lower count (or 0) clears the amount left in the opened pack, as the library does.
    setItems(previous => previous.map(other => other.id === item.id
      ? { ...other, quantity: stepped.quantity, measure_left: stepped.measure_left } : other));
    setStepNote(`${item.name}: ${next} on hand.`);
    try {
      const savedItem = await inventoryApi.setQuantity(item.id, next);
      if (mounted.current) setItems(previous => previous.map(other => other.id === savedItem.id ? savedItem : other));
    } catch (reason) {
      if (mounted.current) {
        setItems(previous => previous.map(other => other.id === item.id && other.quantity === next
          ? { ...other, quantity: before, measure_left: left } : other));
        setStepNote(`${item.name}: ${before ?? 'unknown'} on hand.`);
        toast.error(`Could not change On hand of ${item.name}. ${errText(reason)}`);
      }
    } finally {
      stepLock.current.delete(item.id);
      if (mounted.current) setStepping(new Set(stepLock.current));
    }
  }

  // The amount left in the opened pack, edited in the list. Same rules as the stepper: one request per item at a time,
  // the value changes at once and goes back when the save fails. Returns false when another change of the item is saving.
  function changeOpenAmount(item: InventoryItem, left: number | null): boolean {
    if (stepLock.current.has(item.id)) return false;
    const before = item.measure_left ?? null;
    if (before === left) return true;
    stepLock.current.add(item.id);
    setStepping(new Set(stepLock.current));
    setItems(previous => previous.map(other => other.id === item.id ? { ...other, measure_left: left } : other));
    const unit = item.measure_unit;
    const describe = (amount: number | null) => amount === null
      ? 'no amount recorded for the opened pack' : `${amount} of ${item.measure_full} ${unit} left in the opened pack`;
    setStepNote(`${item.name}: ${describe(left)}.`);
    void (async () => {
      try {
        const savedItem = await inventoryApi.setOpenAmount(item.id, left);
        if (mounted.current) setItems(previous => previous.map(other => other.id === savedItem.id ? savedItem : other));
      } catch (reason) {
        if (mounted.current) {
          setItems(previous => previous.map(other => other.id === item.id && (other.measure_left ?? null) === left
            ? { ...other, measure_left: before } : other));
          setStepNote(`${item.name}: ${describe(before)}.`);
          toast.error(`Could not change Open pack of ${item.name}. ${errText(reason)}`);
        }
      } finally {
        stepLock.current.delete(item.id);
        if (mounted.current) setStepping(new Set(stepLock.current));
      }
    })();
    return true;
  }

  // Keyboard shortcuts of the list: / searches and n adds an item.
  const listBlocked = editor !== null || review !== null || manage;
  const canAdd = !loading && !error;
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.repeat) return;
      const action = shortcutFor({
        key: event.key, ctrlKey: event.ctrlKey, metaKey: event.metaKey, altKey: event.altKey,
        isComposing: event.isComposing, defaultPrevented: event.defaultPrevented,
        target: event.target as HTMLElement | null,
      }, listBlocked);
      if (action === 'search' && searchInput.current) {
        event.preventDefault();
        searchInput.current.focus();
        searchInput.current.select();
      } else if (action === 'add' && canAdd) {
        event.preventDefault();
        returnTo.current = null;
        setCopy(null);
        setEditor('new');
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [listBlocked, canAdd]);

  useEffect(() => {
    try { localStorage.setItem(preference, `${sort.field}:${sort.direction}`); }
    catch { /* The current view still retains the selection when storage is unavailable. */ }
  }, [sort]);

  useLayoutEffect(() => {
    if (editor !== null || review !== null || focusAfterClose.current === undefined) return;
    const id = focusAfterClose.current;
    focusAfterClose.current = undefined;
    const button = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('button[data-item-id]') ?? [])
      .find(node => node.dataset.itemId === id);
    (button ?? addButton.current)?.focus();
  }, [editor, review]);

  const wasManaging = useRef(false);
  useLayoutEffect(() => {
    if (manage) { wasManaging.current = true; return; }
    if (!wasManaging.current) return;
    wasManaging.current = false;
    (manageButton.current ?? addButton.current)?.focus();
  }, [manage]);

  function closeEditor(focusId: string | null) {
    focusAfterClose.current = focusId;
    setEditor(null);
  }

  function saved(item: InventoryItem) {
    setItems(previous => [item, ...previous.filter(other => other.id !== item.id)]);
    if (!matchingItems([item], query, '').length) setQuery('');
    if (noPrice && item.price_cents != null) setNoPrice(false);
    if (noCount && item.quantity != null) setNoCount(false);
    if (effectiveCategory && !sameName(item.category, effectiveCategory)) chooseCategory('');
    closeEditor(item.id);
    loadViewData();
    toast.info('Item saved.');
  }

  function savedMany(saved: InventoryItem[]) {
    setItems(previous => [...saved, ...previous.filter(other => !saved.some(item => item.id === other.id))]);
    setQuery('');
    chooseCategory('');
    setNoPrice(false);
    setNoCount(false);
    focusAfterClose.current = null;
    setReview(null);
    loadViewData();
    toast.info(saved.length === 1 ? 'Item saved.' : `${saved.length} items saved.`);
  }

  function removed(id: string) {
    setItems(previous => previous.filter(item => item.id !== id));
    closeEditor(null);
    toast.info('Item deleted.');
  }

  return (
    <div className="settings workshop">
      <header className="lang-head" data-tauri-drag-region>
        <div className="lang-title">
          <h1>Workshop inventory</h1>
        </div>
        {!editor && !review && !manage && <div className="row">
          <button className="btn" disabled={loading || !!error} onClick={() => setReview('new')}>
            <Camera size={15} aria-hidden="true" /> Add from photo
          </button>
          <button className="btn primary" ref={addButton} disabled={loading || !!error}
            onClick={() => { returnTo.current = null; setCopy(null); setEditor('new'); }}
            title="Add item (press n)" aria-keyshortcuts="n">
            <Plus size={15} aria-hidden="true" /> Add item
          </button>
        </div>}
      </header>
      <div className="settings-body workshop-body">
        {manage ? <WorkshopCategories rows={chips.map(chip => ({
            ...chip, columns: categoryInfos.find(info => sameName(info.name, chip.name))?.columns ?? null }))}
          items={items} currency={currency} onColumns={(name, next) => changeColumns(next, name)} onChanged={reloadAll}
          onRenamed={(from, to) => { if (sameName(category, from)) chooseCategory(to); }}
          onDone={() => setManage(false)} /> : review ? <WorkshopPhotoReview categories={categories} initial={review === 'new' ? null : review}
          onCancel={() => { focusAfterClose.current = null; setReview(null); }} onSaved={savedMany} /> : editor ? <WorkshopItemEditor key={editor === 'new' ? `new:${copy?.seed ?? 0}` : editor.id}
          item={editor === 'new' ? null : editor} categories={categories} currency={currency}
          initialForm={editor === 'new' ? copy?.form : undefined} copyOf={editor === 'new' ? copy?.of : undefined}
          onDuplicate={item => { setCopy({ form: duplicateForm(item), of: item.name, seed: Date.now() }); setEditor('new'); }}
          onCancel={() => closeEditor(returnTo.current)} onSaved={saved} onRemoved={removed}
          onReviewMany={(file, drafts) => { setEditor(null); setReview({ file, drafts }); }} /> : (
          <>
            {loading && <p role="status">Loading inventory…</p>}
            {error && <div className="panel"><p className="error-text" role="alert">{error}</p>
              <button className="btn small" onClick={() => setRetry(n => n + 1)}>Retry</button></div>}
            {!loading && !error && (items.length === 0 ? (
              <div className="workshop-empty panel">
                <Boxes size={36} strokeWidth={1.4} aria-hidden="true" />
                <h2>Your inventory is empty</h2>
                <p>Add your first item from a product label or enter its details by hand.</p>
                <button type="button" className="btn small" ref={manageButton} onClick={() => setManage(true)}>
                  <Tags size={14} aria-hidden="true" /> Manage categories
                </button>
              </div>
            ) : (
              <div className="workshop-catalogue">
                <div className="workshop-chips" role="group" aria-label="Filter by category">
                  <button type="button" className="workshop-chip" aria-pressed={effectiveCategory === ''}
                    onClick={() => chooseCategory('')}>All <span className="workshop-chip-count">{items.length}</span></button>
                  {chips.map(chip => <button key={chip.name} type="button" className="workshop-chip"
                    aria-pressed={effectiveCategory === chip.name} onClick={() => chooseCategory(chip.name)}>
                    {chip.name} <span className="workshop-chip-count">{chip.count}</span>
                  </button>)}
                </div>
                <div className="workshop-tools">
                  <label className="workshop-filter"><Search size={14} aria-hidden="true" />
                    <input type="search" aria-label="Search inventory" placeholder="Search by name, colour or part number (press /)"
                      ref={searchInput} aria-keyshortcuts="/" title="Press / to search"
                      value={query} onChange={e => setQuery(e.target.value)} />
                  </label>
                  <WorkshopColumnPicker scopeLabel={effectiveCategory || 'All categories'} config={columnConfig}
                    shown={columns} onChange={changeColumns} />
                  <button type="button" className="btn small" ref={manageButton} onClick={() => setManage(true)}>
                    <Tags size={14} aria-hidden="true" /> Manage categories
                  </button>
                  <div className="workshop-flags">
                    <button type="button" className="btn small workshop-toggle" aria-pressed={noPrice}
                      onClick={() => setNoPrice(value => !value)}>No price</button>
                    <button type="button" className="btn small workshop-toggle" aria-pressed={noCount}
                      onClick={() => setNoCount(value => !value)}>No count</button>
                  </div>
                  <p className="workshop-count" role="status">
                    {visible.length === items.length ? items.length : `${visible.length} of ${items.length}`} {items.length === 1 ? 'item' : 'items'}
                  </p>
                </div>
                {visible.length === 0 ? <div className="panel">
                  <p>{query || noPrice || noCount ? 'No items match this search.' : `No items in ${effectiveCategory} yet.`}</p>
                  <button className="btn small" onClick={() => { setQuery(''); chooseCategory(''); setNoPrice(false); setNoCount(false); }}>Clear filters</button>
                </div> : (
                  <>
                    <WorkshopInventoryTable tableRef={list} items={visible} columns={columns} currency={currency}
                      sort={shownSort} onSort={field => setSort(nextListSort(shownSort, field))}
                      onOpen={item => { returnTo.current = item.id; setEditor(item); }}
                      onStep={(item, delta) => void stepItem(item, delta)} stepping={stepping}
                      onOpenAmount={changeOpenAmount} />
                    <p className="sr-only" role="status">{stepNote}</p>
                    <footer className="workshop-footer">
                      <p>{[`${totals.count} ${totals.count === 1 ? 'item' : 'items'}`,
                        totals.withoutCount ? `${totals.withoutCount} with no count` : '',
                        totals.unpriced ? `${totals.unpriced} with no price` : ''].filter(Boolean).join(' · ')}</p>
                      {totals.unpriced < totals.count && <p className="workshop-footer-value">
                        Stock value <strong>{formatMoney(totals.stockValueCents, currency)}</strong> (priced items only)
                      </p>}
                    </footer>
                  </>
                )}
              </div>
            ))}
          </>
        )}
      </div>
    </div>
  );
}
