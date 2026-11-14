import { useEffect, useMemo, useRef, useState } from 'react';
import { Boxes, ChevronRight, Plus, Search } from 'lucide-react';
import { errText } from '../api';
import { inventoryApi, type InventoryItem } from '../inventoryApi';
import { useToast } from './Toast';
import WorkshopItemEditor from './WorkshopItemEditor';
import { matchingItems } from './workshopForm';
import './workshop.css';

export default function WorkshopInventory() {
  const toast = useToast();
  const [items, setItems] = useState<InventoryItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const [editor, setEditor] = useState<InventoryItem | 'new' | null>(null);
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('');
  const addButton = useRef<HTMLButtonElement>(null);
  const returnTo = useRef<string | null>(null);
  const list = useRef<HTMLTableElement>(null);

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
  const visible = useMemo(() => matchingItems(items, query, effectiveCategory), [items, query, effectiveCategory]);

  function closeEditor(focusId: string | null) {
    setEditor(null);
    requestAnimationFrame(() => {
      const button = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('button[data-item-id]') ?? [])
        .find(node => node.dataset.itemId === focusId);
      (button ?? addButton.current)?.focus();
    });
  }

  function saved(item: InventoryItem) {
    setItems(previous => [item, ...previous.filter(other => other.id !== item.id)]);
    setQuery('');
    setCategory('');
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
          <div className="eyebrow">Workshop</div>
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
            {loading && <p role="status">Load inventory…</p>}
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
                    <input type="search" aria-label="Search inventory" placeholder="Search inventory"
                      value={query} onChange={e => setQuery(e.target.value)} />
                  </label>
                  <select className="select" aria-label="Filter by category" value={effectiveCategory}
                    onChange={e => setCategory(e.target.value)}>
                    <option value="">All categories</option>
                    {categories.map(c => <option key={c}>{c}</option>)}
                  </select>
                </div>
                <p className="workshop-count" role="status">
                  {visible.length === items.length ? items.length : `${visible.length} of ${items.length}`} {items.length === 1 ? 'item' : 'items'}
                </p>
                {visible.length === 0 ? <div className="panel">
                  <p>No items match this search.</p>
                  <button className="btn small" onClick={() => { setQuery(''); setCategory(''); }}>Clear filters</button>
                </div> : (
                  <div className="workshop-table">
                    <table ref={list} aria-label="Workshop inventory items">
                      <thead><tr><th scope="col">Item</th><th scope="col">Category</th><th scope="col">On hand</th></tr></thead>
                      <tbody>{visible.map(item => (
                        <tr key={item.id}>
                          <th scope="row"><button className="workshop-item-name" data-item-id={item.id} onClick={() => {
                            returnTo.current = item.id; setEditor(item);
                          }}>{item.name}<ChevronRight size={12} aria-hidden="true" /></button>
                            {(item.brand || item.part_number) && <span className="workshop-item-sub">{[item.brand, item.part_number].filter(Boolean).join(' · ')}</span>}
                          </th>
                          <td>{item.category}</td>
                          <td className="workshop-stock">{item.quantity?.toLocaleString() ?? <span className="muted">Unknown</span>}</td>
                        </tr>
                      ))}</tbody>
                    </table>
                  </div>
                )}
              </div>
            ))}
          </>
        )}
      </div>
    </div>
  );
}
