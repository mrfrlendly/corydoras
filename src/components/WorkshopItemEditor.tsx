import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Camera, Trash2 } from 'lucide-react';
import { errText } from '../api';
import { inventoryApi, type InventoryItem } from '../inventoryApi';
import { useConfirmRow } from './useConfirmRow';
import WorkshopSupplier, { type SearchState } from './WorkshopSupplier';
import { blankForm, editForm, LABEL_CATEGORIES, reviewLabel, saveDraft, validateItemPhoto, type WorkshopForm } from './workshopForm';
import './workshopPhotoEditor.css';

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
  ['colour', 'Colour', 120, 'e.g. Translucent orange'],
  ['dimensions', 'Dimensions', 2048, 'e.g. Length 20 mm, diameter 4 mm'],
] as const;

type PhotoChange = File | 'remove' | undefined;
const photoFormats = 'image/jpeg,image/png,image/webp,.jpg,.jpeg,.png,.webp';

function PhotoPicker({ label, disabled, onPick, help }: {
  label: string; disabled: boolean; onPick: (file: File) => void; help?: string;
}) {
  return <label className={`btn small workshop-file ${disabled ? 'disabled' : ''}`}>
    <input className="sr-only" type="file" accept={photoFormats} disabled={disabled} aria-describedby={help}
      onChange={event => {
        const file = event.currentTarget.files?.[0];
        event.currentTarget.value = '';
        if (file) onPick(file);
      }} />{label}
  </label>;
}

function usePhotoPreview(key: string | null, change: PhotoChange) {
  const [preview, setPreview] = useState({ src: '', error: '' });
  useEffect(() => {
    setPreview({ src: '', error: '' });
    if (change === 'remove') return;
    if (change) {
      const src = URL.createObjectURL(change);
      setPreview({ src, error: '' });
      return () => URL.revokeObjectURL(src);
    }
    if (!key) return;
    let current = true;
    inventoryApi.photo(key, false)
      .then(src => { if (current) setPreview({ src, error: '' }); })
      .catch(e => { if (current) setPreview({ src: '', error: errText(e) }); });
    return () => { current = false; };
  }, [key, change]);
  return preview;
}

export default function WorkshopItemEditor({ item, categories, onSaved, onRemoved, onCancel }: Props) {
  const [form, setForm] = useState<WorkshopForm>(() => item ? editForm(item) : blankForm());
  const [busy, setBusy] = useState<'idle' | 'attach' | 'photo' | 'save' | 'delete'>('idle');
  const [autoSearch, setAutoSearch] = useState(true);
  const [manualLink, setManualLink] = useState(false);
  const [searchState, setSearchState] = useState<SearchState>({ status: 'idle' });
  const [photoChange, setPhotoChange] = useState<PhotoChange>(undefined);
  const [labelFile, setLabelFile] = useState<File | null>(null);
  const [labelRead, setLabelRead] = useState(false);
  const preview = usePhotoPreview(item?.photo_key ?? null, photoChange);
  const hasPhoto = photoChange === 'remove' ? false : Boolean(photoChange || item?.photo_key);
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

  async function choosePhoto(file: File, analyse: boolean) {
    setBusy(analyse ? 'photo' : 'attach');
    setPhotoError('');
    try {
      await validateItemPhoto(file);
      if (!mounted.current) return;
      if (!analyse) {
        setPhotoChange(file);
        return;
      }
      if (!hasPhoto || photoChange === labelFile) setPhotoChange(file);
      setLabelFile(file);
      searchRequest.current += 1;
      setSearchState({ status: 'idle' });
      const label = await inventoryApi.readLabel(file);
      if (!mounted.current) return;
      const reviewed = reviewLabel(form, label);
      setForm(reviewed);
      setLabelRead(true);
      if (autoSearch && (reviewed.name.trim() || reviewed.part_number.trim())) void search(reviewed);
    } catch (e) {
      if (mounted.current) setPhotoError(errText(e));
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
      const saved = await inventoryApi.save(item?.id ?? null, saveDraft(form), photoChange);
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
    <section className="panel workshop-photo workshop-editor-photo" aria-labelledby="item-photo-heading">
      <h3 id="item-photo-heading"><Camera size={16} aria-hidden="true" /> Item photo</h3>
      <div className="workshop-editor-photo-row">
        {preview.src && <img alt={labelFile && photoChange === labelFile ? 'Selected product label' : 'Item photo'} src={preview.src} />}
        <div>
          <div className="row">
            <PhotoPicker label={hasPhoto ? 'Replace item photo' : 'Add item photo'}
              disabled={busy !== 'idle'} onPick={file => void choosePhoto(file, false)} help="item-photo-help" />
            {hasPhoto && <button type="button" className="link-btn small" disabled={busy !== 'idle'}
              onClick={() => { setPhotoChange('remove'); setPhotoError(''); }}>Remove photo</button>}
            {item?.photo_key && photoChange !== undefined && <button type="button" className="link-btn small"
              disabled={busy !== 'idle'} onClick={() => { setPhotoChange(undefined); setPhotoError(''); }}>Restore saved photo</button>}
          </div>
          <p className="small muted" id="item-photo-help">JPEG, PNG, or WebP, up to 12 MiB.</p>
          {preview.error && <p className="small error-text" role="alert">Cannot show the saved photo. {preview.error}</p>}
        </div>
      </div>
      {!item && !labelRead && <div className="workshop-label-options">
        <p className="small" id="photo-help">Use a label photo to fill in the item details with your analysis provider.</p>
        <PhotoPicker label="Choose label photo" disabled={busy !== 'idle'}
          onPick={file => void choosePhoto(file, true)} help="photo-help" />
        <label className="check small"><input type="checkbox" checked={autoSearch} disabled={busy !== 'idle'}
          onChange={e => setAutoSearch(e.target.checked)} /> Search suppliers after label analysis</label>
        {autoSearch && <p className="small muted">The search sends the product identifiers to supplier catalogues.</p>}
      </div>}
      {busy === 'photo' && <p className="small" role="status">Label analysis in progress…</p>}
      {photoError && <p className="small error-text" role="alert">{photoError}</p>}
      {labelRead && <p className="small selectable" role="status">Review the values from {labelFile?.name}. Enter the quantity on hand.</p>}
    </section>
    <fieldset disabled={busy !== 'idle'} className="workshop-fields">
      <section className="panel" aria-labelledby="item-details-heading">
        <h3 id="item-details-heading">Item details</h3>
        <div className="workshop-grid">
          {fields.map(([key, title, max, placeholder]) => <label key={key} className="field">
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
