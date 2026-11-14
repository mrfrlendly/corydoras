import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Camera, Trash2 } from 'lucide-react';
import { errText } from '../api';
import { inventoryApi, type InventoryItem } from '../inventoryApi';
import { useConfirmRow } from './useConfirmRow';
import WorkshopSupplier, { type SearchState } from './WorkshopSupplier';
import { blankForm, editForm, LABEL_CATEGORIES, reviewLabel, saveDraft, type WorkshopForm } from './workshopForm';

interface Props {
  item: InventoryItem | null;
  categories: string[];
  onSaved: (item: InventoryItem) => void;
  onRemoved: (id: string) => void;
  onCancel: () => void;
}

const identifierFields = new Set(['name', 'brand', 'part_number']);
const fields = [
  ['name', 'Item name', 512, 'e.g. Socket head screws'],
  ['category', 'Category', 120, 'Choose or enter a category'],
  ['brand', 'Brand', 512, ''],
  ['part_number', 'Part number', 512, ''],
  ['dimensions', 'Dimensions', 2048, 'e.g. Length 20 mm, diameter 4 mm'],
] as const;

export default function WorkshopItemEditor({ item, categories, onSaved, onRemoved, onCancel }: Props) {
  const [form, setForm] = useState<WorkshopForm>(() => item ? editForm(item) : blankForm());
  const [busy, setBusy] = useState<'idle' | 'photo' | 'save' | 'delete'>('idle');
  const [autoSearch, setAutoSearch] = useState(true);
  const [manualLink, setManualLink] = useState(false);
  const [searchState, setSearchState] = useState<SearchState>({ status: 'idle' });
  const [photo, setPhoto] = useState<File | null>(null);
  const [preview, setPreview] = useState('');
  const [photoError, setPhotoError] = useState('');
  const [error, setError] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);
  const trigger = useConfirmRow(confirmDelete, () => setConfirmDelete(false));
  const heading = useRef<HTMLHeadingElement>(null);
  const mounted = useRef(false);
  const searchRequest = useRef(0);
  const saving = busy === 'save' || busy === 'delete';
  const options = [...new Set([...LABEL_CATEGORIES, ...categories])];
  const query = [form.brand, form.name, form.part_number].map(s => s.trim()).filter(Boolean).join(' ');

  useEffect(() => {
    mounted.current = true;
    heading.current?.focus();
    return () => { mounted.current = false; };
  }, []);

  useEffect(() => {
    if (!photo) return;
    const url = URL.createObjectURL(photo);
    setPreview(url);
    return () => URL.revokeObjectURL(url);
  }, [photo]);

  function update<K extends keyof WorkshopForm>(key: K, value: WorkshopForm[K]) {
    setForm(previous => ({ ...previous, [key]: value }));
    if (identifierFields.has(key)) {
      searchRequest.current += 1;
      setSearchState({ status: 'idle' });
    }
  }

  async function search(source: WorkshopForm) {
    const request = ++searchRequest.current;
    setSearchState({ status: 'pending' });
    const current = () => mounted.current && request === searchRequest.current;
    try {
      const results = await inventoryApi.search(source.name, source.brand, source.part_number);
      if (current()) setSearchState({ status: 'done', results });
    } catch (e) {
      if (current()) setSearchState({ status: 'error', error: errText(e) });
    }
  }

  async function readPhoto(file?: File) {
    if (!file) return;
    setBusy('photo');
    setPhotoError('');
    searchRequest.current += 1;
    setSearchState({ status: 'idle' });
    try {
      const label = await inventoryApi.readLabel(file);
      if (!mounted.current) return;
      const reviewed = reviewLabel(form, label);
      setForm(reviewed);
      setPhoto(file);
      if (autoSearch && (reviewed.name.trim() || reviewed.part_number.trim())) void search(reviewed);
    } catch (e) {
      if (mounted.current) setPhotoError(`Could not read this label. ${errText(e)} Choose another photo or enter the details by hand.`);
    } finally {
      if (mounted.current) setBusy('idle');
    }
  }

  async function save(e: FormEvent) {
    e.preventDefault();
    if (busy !== 'idle' || confirmDelete || manualLink) return;
    setError('');
    setBusy('save');
    try {
      const saved = await inventoryApi.save(item?.id ?? null, saveDraft(form));
      if (mounted.current) onSaved(saved);
    } catch (e) {
      if (mounted.current) setError(errText(e));
    } finally {
      if (mounted.current) setBusy('idle');
    }
  }

  async function remove() {
    if (!item || busy !== 'idle') return;
    setBusy('delete');
    setError('');
    try {
      await inventoryApi.remove(item.id);
      if (mounted.current) onRemoved(item.id);
    } catch (e) {
      if (mounted.current) setError(errText(e));
    } finally {
      if (mounted.current) setBusy('idle');
    }
  }

  return <form className="workshop-editor" onSubmit={save}>
    <div className="workshop-heading">
      <h2 ref={heading} tabIndex={-1}>{item ? 'Edit item' : 'Add item'}</h2>
      <button type="button" className="btn small" disabled={saving} onClick={onCancel}>Cancel</button>
    </div>
    {!item && <section className="panel workshop-photo" aria-labelledby="label-heading">
      <div>
        <h3 id="label-heading"><Camera size={16} aria-hidden="true" /> Product label</h3>
        {!photo && <>
        <p className="small muted" id="photo-help">Choose a JPEG, PNG, or WebP photo to send it to your analysis provider. You can also enter the details by hand.</p>
        <label className="check small"><input type="checkbox" checked={autoSearch} disabled={busy !== 'idle'}
          onChange={e => setAutoSearch(e.target.checked)} /> Search suppliers after label analysis</label>
        {autoSearch && <p className="small muted">The search sends the product identifiers to supplier catalogues.</p>}
        <label className={`btn small workshop-file ${busy !== 'idle' ? 'disabled' : ''}`}>
          <input className="sr-only" type="file" accept="image/jpeg,image/png,image/webp,.jpg,.jpeg,.png,.webp"
            aria-describedby="photo-help" disabled={busy !== 'idle'} onChange={e => {
              const file = e.currentTarget.files?.[0];
              e.currentTarget.value = '';
              void readPhoto(file);
            }} />Choose label photo
        </label>
        </>}
        {busy === 'photo' && <p className="small" role="status">Label analysis in progress…</p>}
        {photoError && <p className="small error-text" role="alert">{photoError}</p>}
        {photo && <p className="small selectable" role="status">Review the values from {photo.name}. Enter the quantity on hand.</p>}
      </div>
      {preview && <img alt="Selected product label" src={preview} />}
    </section>}
    <fieldset disabled={busy !== 'idle'} className="workshop-fields">
      <section className="panel" aria-labelledby="item-details-heading">
        <h3 id="item-details-heading">Item details</h3>
        <div className="workshop-grid">
          {fields.map(([key, title, max, placeholder]) => <label key={key} className={`field ${key === 'dimensions' ? 'workshop-wide' : ''}`}>
            <span>{title}</span>
            <input required={key === 'name'} list={key === 'category' ? 'workshop-category-options' : undefined}
              maxLength={max} placeholder={placeholder} value={form[key]} onChange={e => update(key, e.target.value)} />
          </label>)}
          <datalist id="workshop-category-options">{options.map(option => <option key={option} value={option} />)}</datalist>
          {([['quantity', 'Quantity on hand'], ['pack_quantity', 'Quantity per pack']] as const).map(([key, title]) =>
            <label className="field" key={key}><span>{title}</span>
              <input type="text" inputMode="numeric" pattern="[0-9]+" title="Enter a whole number." maxLength={16}
                placeholder="Unknown" value={form[key]} onChange={e => update(key, e.target.value)} />
            </label>)}
          <label className="field workshop-wide"><span>Notes</span><textarea rows={2} maxLength={8192} value={form.notes}
            onChange={e => update('notes', e.target.value)} /></label>
        </div>
      </section>
      <WorkshopSupplier selected={form.supplier} query={query} canSearch={!!(form.name.trim() || form.part_number.trim())}
        manual={manualLink} setManual={setManualLink} state={searchState}
        onSearch={() => void search(form)} onSelect={listing => update('supplier', listing)} />
    </fieldset>
    {error && <p className="error-text" role="alert">{error}</p>}
    {confirmDelete && <div className="panel workshop-confirm">
      <p>Delete “{item?.name}” from this inventory?</p>
      <div className="row">
        <button type="button" className="btn small" autoFocus disabled={saving} onClick={() => setConfirmDelete(false)}>Keep item</button>
        <button type="button" className="btn small danger" disabled={saving} onClick={remove}>Confirm deletion</button>
      </div>
    </div>}
    <div className="workshop-actions">
      {item && !confirmDelete && <button type="button" ref={trigger} className="btn small ghost-danger" disabled={busy !== 'idle'}
        onClick={() => setConfirmDelete(true)}><Trash2 size={13} aria-hidden="true" /> Delete item</button>}
      <button type="submit" className="btn primary" disabled={busy !== 'idle' || confirmDelete || manualLink}>Save item</button>
    </div>
  </form>;
}
