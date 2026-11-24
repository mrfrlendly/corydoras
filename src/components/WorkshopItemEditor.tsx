import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Camera, Copy, Trash2 } from 'lucide-react';
import { errText } from '../api';
import { inventoryApi, type InventoryDraft, type InventoryItem, type MeasureUnit, type SupplierListing } from '../inventoryApi';
import { useConfirmRow } from './useConfirmRow';
import { useToast } from './Toast';
import WorkshopColourBadge from './WorkshopColourBadge';
import WorkshopSpecsTable from './WorkshopSpecsTable';
import WorkshopSupplier, { type SearchState } from './WorkshopSupplier';
import { pieceHint } from './workshopActions.ts';
import { formatAmount } from './workshopColumns.ts';
import {
  blankForm, chooseUnit, editForm, estimateLabel, formTotalAmount, fullFromSpecs, isEstimate, LABEL_CATEGORIES, measureUnitOptions,
  reviewLabel, saveDraft, specSummary, validateItemPhoto, withPrice, withSpecs, type WorkshopForm,
} from './workshopForm';
import {
  categoryForSave, fieldInput, fillFromSuggestions, intakeBase, listingPrice, noAuto, ownedFields, priceFill, shouldAutoFill, unambiguousListingPrice,
  type AutoField, type AutoFilled, type ListingPrice, type Note,
} from './workshopIntake.ts';
import './workshopPhotoEditor.css';

interface Props {
  item: InventoryItem | null;
  categories: string[];
  onSaved: (item: InventoryItem) => void;
  onRemoved: (id: string) => void;
  onCancel: () => void;
  /** A photo that shows several items goes to the review list with the drafts already read. */
  onReviewMany: (file: File, drafts: InventoryDraft[]) => void;
  /** Three-letter currency code of the library, for the price label. */
  currency: string;
  /** Start a new item from this one (Duplicate item). Only offered when an item is open. */
  onDuplicate?: (item: InventoryItem) => void;
  /** A new item that starts with these values, for a copy. */
  initialForm?: WorkshopForm;
  /** The name of the item that `initialForm` was copied from. */
  copyOf?: string;
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

/** Wait this long after the last keystroke before asking for category and colour suggestions. */
const suggestDelayMs = 600;

export default function WorkshopItemEditor({ item, categories, onSaved, onRemoved, onCancel, onReviewMany, currency, onDuplicate, initialForm, copyOf }: Props) {
  const toast = useToast();
  const [form, setForm] = useState<WorkshopForm>(() => item ? editForm(item) : initialForm ?? blankForm());
  const [busy, setBusy] = useState<'idle' | 'attach' | 'photo' | 'save' | 'delete'>('idle');
  const [autoSearch, setAutoSearch] = useState(true);
  const [manualLink, setManualLink] = useState(false);
  const [searchState, setSearchState] = useState<SearchState>({ status: 'idle' });
  const [photoChange, setPhotoChange] = useState<PhotoChange>(undefined);
  const [labelFile, setLabelFile] = useState<File | null>(null);
  const [labelRead, setLabelRead] = useState(false);
  const [many, setMany] = useState<{ file: File; drafts: InventoryDraft[] } | null>(null);
  const preview = usePhotoPreview(item?.photo_key ?? null, photoChange);
  const hasPhoto = photoChange === 'remove' ? false : Boolean(photoChange || item?.photo_key);
  const [photoError, setPhotoError] = useState('');
  const [error, setError] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);
  const trigger = useConfirmRow(confirmDelete, () => setConfirmDelete(false));
  const heading = useRef<HTMLHeadingElement>(null);
  const mounted = useRef(false);
  const searchRequest = useRef(0);
  // Category and colour fill-ins of a new item. A field the designer typed in, or an automatic fill they undid, is never filled again.
  const touched = useRef<Record<AutoField, boolean>>({ category: false, colour: false });
  const [categoryNote, setCategoryNote] = useState<Note | null>(null);
  const [colourNote, setColourNote] = useState<Note | null>(null);
  // What the last automatic fill put in each field. While a field still holds it, a later suggestion may replace it.
  const auto = useRef<AutoFilled>(noAuto);
  const formNow = useRef(form);
  formNow.current = form;
  const suggestRequest = useRef(0);
  const lastAsked = useRef('');
  const suggestFailed = useRef(false);
  const saving = busy === 'save' || busy === 'delete';
  const options = [...new Set([...LABEL_CATEGORIES, ...categories])];
  const perPiece = pieceHint(form.price, form.pack_quantity, currency, { unit: form.measure_unit ?? '', full: form.measure_full ?? '' });
  const unit = form.measure_unit ?? '';
  const total = unit ? formTotalAmount(form) : null;
  const labelFull = unit ? fullFromSpecs(form.specs, unit) : null;
  const estimated = isEstimate(form) && (form.price ?? '').trim() !== '';
  const query = [form.brand, form.name, form.part_number].map(s => s.trim()).filter(Boolean).join(' ');

  useEffect(() => {
    mounted.current = true;
    heading.current?.focus();
    return () => { mounted.current = false; };
  }, []);

  // A pause in typing the name, brand or notes of a new item asks for a category and a colour. Name's blur asks at once.
  useEffect(() => {
    if (item) return;
    const timer = window.setTimeout(() => void suggest(), suggestDelayMs);
    return () => window.clearTimeout(timer);
  }, [form.name, form.brand, form.notes]);

  /** Fills an empty colour and a generic category of a new item from the words of the form (and of a chosen listing). */
  async function suggest(listing?: SupplierListing | null) {
    if (item) return;
    // An automatic fill that the designer has not touched counts as empty, so that new words can change it.
    const current = intakeBase(formNow.current, auto.current, touched.current);
    const own = ownedFields(formNow.current, auto.current, touched.current);
    if (!own.category && !own.colour && !shouldAutoFill('category', current.category, touched.current.category, categories)
      && !shouldAutoFill('colour', current.colour, touched.current.colour, categories)) return;
    const input = fieldInput({ ...current, dimensions: current.specs.length ? specSummary(current.specs) : current.dimensions }, listing);
    if (!input.name) return;
    const asked = JSON.stringify(input);
    if (asked === lastAsked.current) return;
    lastAsked.current = asked;
    const request = ++suggestRequest.current;
    try {
      const result = await inventoryApi.suggestFields(input);
      if (!mounted.current || request !== suggestRequest.current) return;
      const filled = fillFromSuggestions(formNow.current, result, touched.current, categories, auto.current);
      auto.current = filled.auto;
      if (filled.categoryNote !== undefined) setCategoryNote(filled.categoryNote);
      if (filled.colourNote !== undefined) setColourNote(filled.colourNote);
      const { patch } = filled;
      if (patch.category === undefined && patch.colour === undefined) return;
      setForm(previous => ({ ...previous, ...patch }));
    } catch (e) {
      // Suggestions are a help. A failure never blocks the form; say so once.
      lastAsked.current = '';
      if (mounted.current && !suggestFailed.current) {
        suggestFailed.current = true;
        toast.error(`Could not suggest a category or colour. ${errText(e).replace(/[.!?]?\s*$/, '.')} You can still save this item.`);
      }
    }
  }

  function undoCategory() {
    if (!categoryNote) return;
    const { previous } = categoryNote;
    touched.current.category = true;
    auto.current = { ...auto.current, category: null };
    setCategoryNote(null);
    setForm(current => ({ ...current, category: previous }));
    document.getElementById('item-category')?.focus();
  }

  function undoColour() {
    if (!colourNote) return;
    const { previous } = colourNote;
    touched.current.colour = true;
    auto.current = { ...auto.current, colour: null };
    setColourNote(null);
    setForm(current => ({ ...current, colour: previous }));
    document.getElementById('item-colour')?.focus();
  }

  /** The estimate that a listing gives, when the price is empty. */
  function fillPrice(fill: ListingPrice | null) {
    if (!fill) return;
    setForm(previous => priceFill(previous, fill) ? { ...previous, ...fill } : previous);
  }

  function chooseListing(listing: SupplierListing | null) {
    update('supplier', listing);
    if (!listing) return;
    fillPrice(listingPrice(listing, currency));
    void suggest(listing);
  }

  function update<K extends keyof WorkshopForm>(key: K, value: WorkshopForm[K]) {
    if (key === 'category' || key === 'colour') touched.current[key as AutoField] = true;
    if (key === 'category') setCategoryNote(null);
    if (key === 'colour') setColourNote(null);
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
      if (current()) {
        setSearchState({ status: 'done', results });
        // Only the clear case fills: one price for the first product, in the library currency.
        fillPrice(unambiguousListingPrice(results, currency));
      }
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
      const labels = await inventoryApi.readLabels(file);
      if (!mounted.current) return;
      if (labels.length > 1) {
        setMany({ file, drafts: labels });
        return;
      }
      const reviewed = reviewLabel(form, labels[0]);
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
      const draft = saveDraft(form);
      const saved = await inventoryApi.save(item?.id ?? null, { ...draft, category: categoryForSave(form, touched.current.category) }, photoChange);
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
    {copyOf && !item && <p className="small muted workshop-copy-note">Copy of {copyOf}. The count and photo are not copied.</p>}
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
      {many && <div className="workshop-many" role="status">
        <p className="small">This photo shows {many.drafts.length} items.</p>
        <button type="button" className="btn small primary" onClick={() => onReviewMany(many.file, many.drafts)}>Open them in the review list</button>
      </div>}
      {labelRead && <p className="small selectable" role="status">Review the values from {labelFile?.name}. Enter the quantity on hand.</p>}
    </section>
    <fieldset disabled={busy !== 'idle'} className="workshop-fields">
      <section className="panel" aria-labelledby="item-details-heading">
        <h3 id="item-details-heading">Item details</h3>
        <div className="workshop-grid">
          {fields.map(([key, title, max, placeholder]) => {
            const input = <input id={key === 'category' ? 'item-category' : key === 'colour' ? 'item-colour' : undefined} required={key === 'name'}
              list={key === 'category' ? 'workshop-category-options' : undefined}
              maxLength={max} placeholder={placeholder} onChange={e => update(key, e.target.value)}
              onBlur={key === 'name' ? () => void suggest() : undefined}
              readOnly={key === 'dimensions' && form.specs.length > 0}
              value={key === 'dimensions' && form.specs.length > 0 ? specSummary(form.specs) : form[key]} />;
            const field = <label key={key} className="field">
              <span>{title}</span>
              {key === 'colour' ? <span className="workshop-input-badge">{input}<WorkshopColourBadge text={form.colour} /></span> : input}
              {key === 'dimensions' && form.specs.length > 0 && <span className="small muted">Generated from the specs below.</span>}
            </label>;
            // The note and its Undo button sit beside the label, not in it, so that a click on Undo does not focus the field.
            const note = key === 'category' ? categoryNote : key === 'colour' ? colourNote : null;
            // Always wrapped, so that the input keeps its place (and focus) when a note appears or goes.
            return key === 'category' || key === 'colour' ? <div key={key} className="workshop-field-cell">
              {field}
              {note && <p className="small muted workshop-autofill" role="status">
                {note.text} <button type="button" className="link-btn small" onClick={key === 'category' ? undoCategory : undoColour}>Undo</button>
              </p>}
            </div> : field;
          })}
          <datalist id="workshop-category-options">{options.map(option => <option key={option} value={option} />)}</datalist>
          {([['quantity', 'Quantity on hand'], ['pack_quantity', 'Quantity per pack']] as const).map(([key, title]) =>
            <label className="field" key={key}><span>{title}</span>
              <input type="text" inputMode="numeric" pattern="[0-9]+" title="Enter a whole number." maxLength={16}
                placeholder="Unknown" value={form[key]} onChange={e => update(key, e.target.value)} />
            </label>)}
          <div className="workshop-price">
            <label className="field"><span>Price per pack ({currency})</span>
              <input type="text" inputMode="decimal" maxLength={20} placeholder="Unknown" value={form.price}
                aria-describedby={[perPiece ? 'item-price-piece' : '', estimated ? 'item-price-estimate' : ''].filter(Boolean).join(' ') || undefined}
                onChange={e => setForm(previous => withPrice(previous, e.target.value))} />
            </label>
            {perPiece && <span className="small muted" id="item-price-piece">{perPiece}</span>}
            {estimated && <span className="small muted" id="item-price-estimate" role="status">
              {estimateLabel(form.price_source)}. Change it if you paid less, and check the pack size.</span>}
          </div>
          <fieldset className="workshop-measure workshop-wide">
            <legend>Measured pack</legend>
            <div className="workshop-measure-row">
              <label className="field"><span>Unit</span>
                <select className="select" value={unit} onChange={e => setForm(previous => chooseUnit(previous, e.target.value as MeasureUnit))}>
                  {measureUnitOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
                </select>
              </label>
              {unit && <>
                <div className="workshop-measure-full">
                  <label className="field"><span>Full pack ({unit})</span>
                    <input type="text" inputMode="numeric" pattern="[0-9]+" title="Enter a whole number." maxLength={16}
                      placeholder="e.g. 1000" value={form.measure_full} onChange={e => update('measure_full', e.target.value)} />
                  </label>
                  {labelFull !== null && form.measure_full.trim() !== String(labelFull) && <button type="button"
                    className="link-btn small" onClick={() => update('measure_full', String(labelFull))}>
                    Use label weight ({formatAmount(labelFull, unit)})</button>}
                </div>
                <label className="field"><span>Left in the opened pack ({unit})</span>
                  <input type="text" inputMode="numeric" pattern="[0-9]+" title="Enter a whole number." maxLength={16}
                    placeholder="Not recorded" value={form.measure_left} onChange={e => update('measure_left', e.target.value)} />
                </label>
              </>}
            </div>
            {unit ? <>
              <p className="small muted">On hand counts whole packs including the opened one.</p>
              <p className="small workshop-measure-total" aria-live="polite">
                {total !== null ? `Total about ${formatAmount(total, unit)}` : ''}</p>
            </> : <p className="small muted">Choose a unit for filament, liquids or cable that is used from a pack.</p>}
          </fieldset>
          <label className="field workshop-wide"><span>Notes</span><textarea rows={2} maxLength={8192} value={form.notes}
            onChange={e => update('notes', e.target.value)} /></label>
        </div>
        <WorkshopSpecsTable specs={form.specs} disabled={busy !== 'idle'} label={form.name || 'this item'}
          onChange={specs => setForm(previous => withSpecs(previous, specs))} />
      </section>
      <WorkshopSupplier selected={form.supplier} query={query} canSearch={!!(form.name.trim() || form.part_number.trim())}
        manual={manualLink} setManual={setManualLink} state={searchState}
        onSearch={() => void search(form)} onSelect={chooseListing} currency={currency} />
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
      {item && onDuplicate && !confirmDelete && <button type="button" className="btn small" disabled={busy !== 'idle'}
        onClick={() => onDuplicate(item)}><Copy size={13} aria-hidden="true" /> Duplicate item</button>}
      {item && !confirmDelete && <button type="button" ref={trigger} className="btn small ghost-danger" disabled={busy !== 'idle'}
        onClick={() => setConfirmDelete(true)}><Trash2 size={13} aria-hidden="true" /> Delete item</button>}
      <button type="submit" className="btn primary" disabled={busy !== 'idle' || confirmDelete || manualLink}>Save item</button>
    </div>
  </form>;
}
