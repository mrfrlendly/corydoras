import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Boxes, Plus, Search } from 'lucide-react';
import { errText } from '../api';
import { inventoryApi, type InventoryItem } from '../inventoryApi';
import { useToast } from './Toast';
import WorkshopItemEditor from './WorkshopItemEditor';
import { matchingItems } from './workshopForm';
import WorkshopInventoryTable from './WorkshopInventoryTable';
import { defaultListSort, nextListSort, parseListSort, sortItems, type ListSort } from './workshopList';
import './workshop.css';

const preference = 'corydoras.workshop.inventory.sort';

function readSort(): ListSort {
  try { return parseListSort(localStorage.getItem(preference)); }
  catch { return defaultListSort; }
}

export default function WorkshopInventory() {
  const toast = useToast();
  const [items, setItems] = useState<InventoryItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const [editor, setEditor] = useState<InventoryItem | 'new' | null>(null);
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('');
  const [sort, setSort] = useState(readSort);
  const addButton = useRef<HTMLButtonElement>(null);
  const returnTo = useRef<string | null>(null);
  const list = useRef<HTMLTableElement>(null);
  const focusAfterClose = useRef<string | null | undefined>(undefined);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError('');
    inventoryApi.list()
      .then(result => { if (active) setItems(result); })
      .catch(reason => { if (active) setError(errText(reason)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [retry]);

  const categories = useMemo(() => [...new Set(items.map(i => i.category))].sort((a, b) => a.localeCompare(b)), [items]);
  const effectiveCategory = categories.includes(category) ? category : '';
  const visible = useMemo(() => sortItems(matchingItems(items, query, effectiveCategory), sort),
    [items, query, effectiveCategory, sort]);
  useEffect(() => {
    try { localStorage.setItem(preference, `${sort.field}:${sort.direction}`); }
    catch { /* The current view still retains the selection when storage is unavailable. */ }
  }, [sort]);

  useLayoutEffect(() => {
    if (editor !== null || focusAfterClose.current === undefined) return;
    const id = focusAfterClose.current;
    focusAfterClose.current = undefined;
    const button = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('button[data-item-id]') ?? [])
      .find(node => node.dataset.itemId === id);
    (button ?? addButton.current)?.focus();
  }, [editor]);

  function closeEditor(focusId: string | null) {
    focusAfterClose.current = focusId;
    setEditor(null);
  }

  function saved(item: InventoryItem) {
    setItems(previous => [item, ...previous.filter(other => other.id !== item.id)]);
    if (!matchingItems([item], query, '').length) setQuery('');
    if (effectiveCategory && item.category !== effectiveCategory) setCategory('');
    closeEditor(item.id);
    toast.info('Item saved.');
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
        {!editor && <button className="btn primary" ref={addButton} disabled={loading || !!error}
          onClick={() => { returnTo.current = null; setEditor('new'); }}>
          <Plus size={15} aria-hidden="true" /> Add item
        </button>}
      </header>
      <div className="settings-body workshop-body">
        {editor ? <WorkshopItemEditor key={editor === 'new' ? 'new' : editor.id}
          item={editor === 'new' ? null : editor} categories={categories}
          onCancel={() => closeEditor(returnTo.current)} onSaved={saved} onRemoved={removed} /> : (
          <>
            {loading && <p role="status">Loading inventory…</p>}
            {error && <div className="panel"><p className="error-text" role="alert">{error}</p>
              <button className="btn small" onClick={() => setRetry(n => n + 1)}>Retry</button></div>}
            {!loading && !error && (items.length === 0 ? (
              <div className="workshop-empty panel">
                <Boxes size={36} strokeWidth={1.4} aria-hidden="true" />
                <h2>Your inventory is empty</h2>
                <p>Add your first item from a product label or enter its details by hand.</p>
              </div>
            ) : (
              <div className="workshop-catalogue">
                <div className="workshop-tools">
                  <label className="workshop-filter"><Search size={14} aria-hidden="true" />
                    <input type="search" aria-label="Search inventory" placeholder="Search by name, colour or part number"
                      value={query} onChange={e => setQuery(e.target.value)} />
                  </label>
                  <select className="select" aria-label="Filter by category" value={effectiveCategory}
                    onChange={e => setCategory(e.target.value)}>
                    <option value="">All categories</option>
                    {categories.map(c => <option key={c}>{c}</option>)}
                  </select>
                  <p className="workshop-count" role="status">
                    {visible.length === items.length ? items.length : `${visible.length} of ${items.length}`} {items.length === 1 ? 'item' : 'items'}
                  </p>
                </div>
                {visible.length === 0 ? <div className="panel">
                  <p>No items match this search.</p>
                  <button className="btn small" onClick={() => { setQuery(''); setCategory(''); }}>Clear filters</button>
                </div> : (
                  <WorkshopInventoryTable tableRef={list} items={visible} sort={sort}
                    onSort={field => setSort(current => nextListSort(current, field))}
                    onOpen={item => { returnTo.current = item.id; setEditor(item); }} />
                )}
              </div>
            ))}
          </>
        )}
      </div>
    </div>
  );
}
