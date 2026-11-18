import { useEffect, useRef, useState } from 'react';
import { Camera } from 'lucide-react';
import { errText } from '../api';
import { inventoryApi, type InventoryDraft, type InventoryItem } from '../inventoryApi';
import { useConfirmRow } from './useConfirmRow';
import WorkshopSpecsTable from './WorkshopSpecsTable';
import { addPack, batchDrafts, editForm, LABEL_CATEGORIES, specSummary, validateItemPhoto, type WorkshopForm } from './workshopForm';
import './workshopPhotoEditor.css';

interface Props {
  categories: string[];
  /** A photo that the editor already read, so the provider is not asked twice. */
  initial: { file: File; drafts: InventoryDraft[] } | null;
  onSaved: (items: InventoryItem[]) => void;
  onCancel: () => void;
}

interface Card { id: number; form: WorkshopForm }

const textFields = [
  ['name', 'Item name', 512], ['category', 'Category', 120], ['brand', 'Brand', 512],
  ['part_number', 'Part number', 512], ['colour', 'Colour', 120],
] as const;

/** One card for each item that a photo shows. The designer edits, discards or saves them together. */
export default function WorkshopPhotoReview({ categories, initial, onSaved, onCancel }: Props) {
  const [file, setFile] = useState<File | null>(initial?.file ?? null);
  const [cards, setCards] = useState<Card[]>(() => (initial?.drafts ?? []).map((draft, id) => ({ id, form: editForm(draft) })));
  const [busy, setBusy] = useState<'idle' | 'read' | 'save'>('idle');
  const [error, setError] = useState('');
  const [preview, setPreview] = useState('');
  const [confirmCancel, setConfirmCancel] = useState(false);
  const trigger = useConfirmRow(confirmCancel, () => setConfirmCancel(false));
  const heading = useRef<HTMLHeadingElement>(null);
  const mounted = useRef(false);
  const counter = useRef(cards.length);
  const options = [...new Set([...LABEL_CATEGORIES, ...categories])];

  useEffect(() => {
    mounted.current = true;
    heading.current?.focus();
    return () => { mounted.current = false; };
  }, []);

  useEffect(() => {
    if (!file) return;
    const src = URL.createObjectURL(file);
    setPreview(src);
    return () => URL.revokeObjectURL(src);
  }, [file]);

  async function pick(chosen: File) {
    setBusy('read');
    setError('');
    try {
      await validateItemPhoto(chosen);
      const drafts = await inventoryApi.readLabels(chosen);
      if (!mounted.current) return;
      setFile(chosen);
      setCards(drafts.map(draft => ({ id: counter.current++, form: editForm(draft) })));
    } catch (e) {
      if (mounted.current) setError(errText(e));
    } finally {
      if (mounted.current) setBusy('idle');
    }
  }

  function change(id: number, patch: Partial<WorkshopForm>) {
    setCards(previous => previous.map(card => card.id === id ? { ...card, form: { ...card.form, ...patch } } : card));
  }

  function bought(card: Card) {
    try {
      change(card.id, { quantity: addPack(card.form).quantity });
      setError('');
    } catch (e) { setError(errText(e)); }
  }

  async function save() {
    if (!file || busy !== 'idle' || cards.length === 0) return;
    setError('');
    setBusy('save');
    try {
      const saved = await inventoryApi.saveBatch(batchDrafts(cards.map(card => card.form)), file);
      if (mounted.current) onSaved(saved);
    } catch (e) {
      if (mounted.current) setError(errText(e));
    } finally {
      if (mounted.current) setBusy('idle');
    }
  }

  const working = busy !== 'idle';
  return <div className="workshop-editor workshop-review">
    <div className="workshop-heading">
      <h2 ref={heading} tabIndex={-1}>Add from photo</h2>
      <button type="button" className="btn small" disabled={busy === 'save'}
        onClick={() => cards.length ? setConfirmCancel(true) : onCancel()} ref={trigger}>Cancel</button>
    </div>
    {confirmCancel && <div className="panel workshop-confirm">
      <p>Discard {cards.length === 1 ? 'the 1 unsaved item' : `the ${cards.length} unsaved items`} from this photo?</p>
      <div className="row">
        <button type="button" className="btn small" autoFocus onClick={() => setConfirmCancel(false)}>Keep reviewing</button>
        <button type="button" className="btn small danger" onClick={onCancel}>Discard items</button>
      </div>
    </div>}
    <section className="panel workshop-photo workshop-editor-photo" aria-labelledby="review-photo-heading">
      <h3 id="review-photo-heading"><Camera size={16} aria-hidden="true" /> Label photo</h3>
      <div className="workshop-editor-photo-row">
        {preview && <img alt="Selected label photo" src={preview} />}
        <div>
          <label className={`btn small workshop-file ${working ? 'disabled' : ''}`}>
            <input className="sr-only" type="file" accept="image/jpeg,image/png,image/webp,.jpg,.jpeg,.png,.webp" disabled={working}
              onChange={event => {
                const chosen = event.currentTarget.files?.[0];
                event.currentTarget.value = '';
                if (chosen) void pick(chosen);
              }} />{file ? 'Choose another photo' : 'Choose label photo'}
          </label>
          <p className="small muted">JPEG, PNG, or WebP, up to 12 MiB. Your analysis provider reads the photo and returns one draft per item. Nothing is saved until you press Save.</p>
          {busy === 'read' && <p className="small" role="status">Label analysis in progress…</p>}
        </div>
      </div>
    </section>
    {cards.length > 0 && <p className="small selectable" role="status">
      Review {cards.length === 1 ? 'the item' : `the ${cards.length} items`}. Quantity on hand is blank: enter it, or press “I bought one pack”.
    </p>}
    {cards.map((card, index) => <section key={card.id} className="panel workshop-card" aria-label={`Item ${index + 1}`}>
      <div className="workshop-grid">
        {textFields.map(([key, title, max]) => <label key={key} className="field">
          <span>{title}</span>
          <input required={key === 'name'} list={key === 'category' ? `review-categories-${card.id}` : undefined} maxLength={max}
            disabled={working} value={card.form[key]} onChange={e => change(card.id, { [key]: e.target.value })} />
        </label>)}
        <datalist id={`review-categories-${card.id}`}>{options.map(option => <option key={option} value={option} />)}</datalist>
        {([['quantity', 'Quantity on hand'], ['pack_quantity', 'Quantity per pack']] as const).map(([key, title]) =>
          <label className="field" key={key}><span>{title}</span>
            <input type="text" inputMode="numeric" pattern="[0-9]+" title="Enter a whole number." maxLength={16} placeholder="Unknown"
              disabled={working} value={card.form[key]} onChange={e => change(card.id, { [key]: e.target.value })} />
          </label>)}
        <label className="field workshop-wide"><span>Dimensions</span>
          <input readOnly={card.form.specs.length > 0} maxLength={2048} disabled={working}
            value={card.form.specs.length > 0 ? specSummary(card.form.specs) : card.form.dimensions}
            onChange={e => change(card.id, { dimensions: e.target.value })} />
          {card.form.specs.length > 0 && <span className="small muted">Generated from the specs below.</span>}
        </label>
      </div>
      <WorkshopSpecsTable specs={card.form.specs} disabled={working} label={card.form.name || `item ${index + 1}`}
        onChange={specs => change(card.id, { specs, dimensions: specs.length ? specSummary(specs) : card.form.dimensions })} />
      <div className="workshop-actions">
        <button type="button" className="link-btn small" disabled={working}
          onClick={() => setCards(previous => previous.filter(other => other.id !== card.id))}>Discard</button>
        <button type="button" className="btn small" disabled={working || !card.form.pack_quantity.trim()}
          onClick={() => bought(card)}>I bought one pack</button>
      </div>
    </section>)}
    {error && <p className="error-text" role="alert">{error}</p>}
    {file && <div className="workshop-actions">
      <button type="button" className="btn primary" disabled={working || cards.length === 0} onClick={() => void save()}>
        {cards.length === 1 ? 'Save 1 item' : `Save ${cards.length} items`}
      </button>
    </div>}
  </div>;
}
