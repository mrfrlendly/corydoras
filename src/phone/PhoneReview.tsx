import { useState } from 'react';
import { addPack, LABEL_CATEGORIES, removeSpecRow, sourceLabel, specSummary, withPrice, type WorkshopForm } from '../components/workshopForm';
import { errText } from './errText';

export interface Card { id: number; form: WorkshopForm }

interface Props {
  cards: Card[];
  categories: string[];
  disabled: boolean;
  onChange: (id: number, form: WorkshopForm) => void;
  onDiscard: (id: number) => void;
}

// Codes and names as printed: no autocorrect, no capitals, no spelling marks.
const asPrinted = { autoCorrect: 'off', autoCapitalize: 'off', spellCheck: false } as const;

const textFields = [
  ['name', 'Item name', 512, {}],
  ['category', 'Category', 120, {}],
  ['brand', 'Brand', 512, asPrinted],
  ['part_number', 'Part number', 512, asPrinted],
  ['colour', 'Colour', 120, asPrinted],
] as const;

/** One card per item. Specs can be removed here; their values are edited on the Mac. */
export default function PhoneReview({ cards, categories, disabled, onChange, onDiscard }: Props) {
  const [packError, setPackError] = useState<{ id: number; text: string } | null>(null);
  const options = [...new Set([...LABEL_CATEGORIES, ...categories])];

  return <>
    <datalist id="phone-categories">{options.map(option => <option key={option} value={option} />)}</datalist>
    {cards.map((card, index) => {
      const { form } = card;
      const set = (patch: Partial<WorkshopForm>) => onChange(card.id, { ...form, ...patch });
      return <section key={card.id} className="card" aria-label={`Item ${index + 1}`}>
        <h2 className="card-title">{form.name.trim() || `Item ${index + 1}`}</h2>
        {textFields.map(([key, title, max, extra]) => <label key={key} className="field">
          <span>{title}</span>
          <input {...extra} required={key === 'name'} maxLength={max} disabled={disabled}
            list={key === 'category' ? 'phone-categories' : undefined} enterKeyHint="next"
            value={form[key]} onChange={e => set({ [key]: e.target.value })} />
        </label>)}
        <div className="pair">
          {([['quantity', 'On hand'], ['pack_quantity', 'Per pack']] as const).map(([key, title]) =>
            <label className="field" key={key}><span>{title}</span>
              <input type="text" inputMode="numeric" pattern="[0-9]*" maxLength={16} placeholder="Unknown" enterKeyHint="done"
                disabled={disabled} value={form[key]} onChange={e => set({ [key]: e.target.value })} />
            </label>)}
          <label className="field"><span>Price per pack</span>
            <input {...asPrinted} type="text" inputMode="decimal" maxLength={16} placeholder="Unknown" enterKeyHint="done"
              disabled={disabled} value={form.price} onChange={e => onChange(card.id, withPrice(form, e.target.value))} />
          </label>
        </div>
        <button type="button" className="btn" disabled={disabled || !form.pack_quantity.trim()} onClick={() => {
          try {
            onChange(card.id, addPack(form));
            setPackError(null);
          } catch (e) { setPackError({ id: card.id, text: errText(e) }); }
        }}>I bought one pack</button>
        {packError?.id === card.id && <p className="error" role="alert">{packError.text}</p>}
        <label className="field"><span>Dimensions</span>
          <input readOnly={form.specs.length > 0} maxLength={2048} disabled={disabled}
            value={form.specs.length > 0 ? specSummary(form.specs) : form.dimensions}
            onChange={e => set({ dimensions: e.target.value })} />
        </label>
        {form.specs.length > 0 && <ul className="specs" aria-label="Specs">
          {form.specs.map((spec, i) => <li key={`${spec.role}-${spec.name}-${spec.value}-${i}`}>
            <div>
              <span className="spec-value">{specSummary([spec])}</span>
              <span className="spec-source">{sourceLabel[spec.source]}</span>
            </div>
            <button type="button" className="btn small" disabled={disabled}
              aria-label={`Remove ${specSummary([spec])}`}
              onClick={() => onChange(card.id, removeSpecRow(form, i))}>Remove</button>
          </li>)}
        </ul>}
        {form.specs.length > 0 && <p className="hint">Wrong value? Remove it here; edit values on the Mac.</p>}
        <button type="button" className="btn quiet" disabled={disabled} onClick={() => onDiscard(card.id)}>Discard</button>
      </section>;
    })}
  </>;
}
