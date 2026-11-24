import { useId, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { ArrowDown, ArrowUp, ChevronsUpDown, Minus, Plus } from 'lucide-react';
import type { ColumnId, InventoryItem } from '../inventoryApi';
import WorkshopThumbnail from './WorkshopThumbnail';
import {
  captionFields, columnDefs, formatMoney, formatUnitPrice, fractionLeft, isEstimatedPrice, isMeasured, openPackText, stockValueCents,
  unitPriceCents,
} from './workshopColumns';
import { estimateLabel } from './workshopForm';
import { canDecrease, checkOpenAmount } from './workshopActions';
import { swatchColours, type ListSort, type SortField } from './workshopList';

interface Props {
  tableRef: RefObject<HTMLTableElement | null>;
  items: InventoryItem[];
  /** The columns after Item, in order. */
  columns: ColumnId[];
  /** Three-letter currency code of the library. */
  currency: string;
  /** The sort that the list shows: a hidden sort column has already fallen back to name. */
  sort: ListSort;
  onSort: (field: SortField) => void;
  onOpen: (item: InventoryItem) => void;
  /** The - and + buttons of On hand: change the count by `delta`. */
  onStep: (item: InventoryItem, delta: number) => void;
  /** Items whose count is being saved. Their buttons ignore clicks. */
  stepping: ReadonlySet<string>;
  /** Save the amount left in the opened pack (null clears it). Returns false when another change of the item is still saving. */
  onOpenAmount: (item: InventoryItem, left: number | null) => boolean;
}

/** A missing value: a dash for sighted users and a word for screen readers. */
function Missing({ label, title }: { label: string; title?: string }) {
  return <span className="workshop-unknown" title={title}><span aria-hidden="true">—</span><span className="sr-only">{label}</span></span>;
}

function Colour({ text }: { text: string }) {
  if (!text) return <Missing label="Unknown" />;
  const colours = swatchColours(text);
  return <span className="workshop-colour">
    {colours.length > 0 && <span className="workshop-swatch" aria-hidden="true">
      {colours.map((colour, index) => <i key={index} style={{ backgroundColor: colour }} />)}
    </span>}
    {text}
  </span>;
}

/** One line with an ellipsis. The full text is in the tooltip. */
function Clipped({ text }: { text: string }) {
  const value = text.trim();
  return value ? <span className="workshop-clip" title={value}>{value}</span> : <Missing label="None" />;
}

function plain(text: string | null | undefined): ReactNode {
  return text?.trim() ? text : <Missing label="None" />;
}

function money(cents: number | null, currency: string, format: typeof formatMoney, label: string): ReactNode {
  return cents === null ? <Missing label={label} /> : format(cents, currency);
}

/** A price that is an estimate from a supplier listing carries a small "est." tag. */
function estimated(item: InventoryItem, value: ReactNode): ReactNode {
  if (!isEstimatedPrice(item)) return value;
  return <>{value}<span className="workshop-estimate" title={estimateLabel(item.price_source)}>est.</span></>;
}

/** On hand with - and + buttons. The buttons stay focusable when they are off, so that focus is not lost at 0 or while saving. */
function Stepper({ item, pending, onStep }: { item: InventoryItem; pending: boolean; onStep: Props['onStep'] }) {
  const quantity = item.quantity ?? null;
  const label = columnDefs.quantity.label;
  const minusOff = pending || !canDecrease(quantity);
  return <span className="workshop-stepper">
    <button type="button" className="workshop-step" aria-label={`Decrease ${label} of ${item.name}`}
      aria-disabled={minusOff || undefined} onClick={() => { if (!minusOff) onStep(item, -1); }}>
      <Minus size={13} aria-hidden="true" />
    </button>
    <span className="workshop-step-value">
      {quantity === null ? <span className="workshop-unknown">Unknown</span> : quantity.toLocaleString()}
    </span>
    <button type="button" className="workshop-step" aria-label={`Increase ${label} of ${item.name}`}
      aria-disabled={pending || undefined} onClick={() => { if (!pending) onStep(item, 1); }}>
      <Plus size={13} aria-hidden="true" />
    </button>
  </span>;
}

/** The opened pack: "100 of 1000 g" with a thin gauge. Click the amount to change it; Enter saves, Escape cancels. */
function OpenAmount({ item, pending, onSave }: { item: InventoryItem; pending: boolean; onSave: Props['onOpenAmount'] }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState('');
  const [error, setError] = useState('');
  const button = useRef<HTMLButtonElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const refocus = useRef(false);
  // Set at once when the editor closes, so that the blur that removing the input may cause cannot undo the focus return.
  const closed = useRef(true);
  const errorId = useId();
  const full = item.measure_full ?? null;
  const unit = item.measure_unit;
  const left = item.measure_left ?? null;

  // The input takes focus when it opens, and the amount button takes it back when it closes. The row may move when the
  // list is sorted by this column, so the focus is set after the render.
  useLayoutEffect(() => {
    if (editing) input.current?.select();
    else if (refocus.current) { refocus.current = false; button.current?.focus(); }
  }, [editing]);

  if (!unit || full === null || full < 1) return <Missing label="No measured pack" />;
  // With no pack on hand there is no opened pack to record. An unknown count still allows the editor.
  if (item.quantity === 0) return <Missing label="No pack on hand. Add a pack on hand first" title="Add a pack on hand first" />;

  function open() {
    setText(String(left ?? full));
    setError('');
    closed.current = false;
    setEditing(true);
  }
  function close(returnFocus: boolean) {
    closed.current = true;
    refocus.current = returnFocus;
    setEditing(false);
    setError('');
  }
  function commit() {
    const check = checkOpenAmount(text, full as number, unit);
    if (!check.ok) { setError(check.error); return; }
    if (!onSave(item, check.left)) { setError('Another change to this item is still saving. Try again in a moment.'); return; }
    close(true);
  }

  const fraction = fractionLeft(item);
  const percent = fraction === null ? null : Math.round(Math.min(1, Math.max(0, fraction)) * 100);
  const shown = openPackText(item);
  return <span className="workshop-open">
    {editing ? <span className="workshop-open-edit">
      <input ref={input} type="text" inputMode="numeric" maxLength={16} value={text} autoComplete="off"
        aria-label={`Left in the opened pack of ${item.name}, in ${unit}`}
        aria-invalid={error ? true : undefined} aria-describedby={error ? errorId : undefined}
        onChange={event => { setText(event.target.value); setError(''); }}
        onKeyDown={event => {
          if (event.key === 'Enter') { event.preventDefault(); commit(); }
          else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(true); }
        }}
        onBlur={() => { if (!closed.current) close(false); }} />
      <span className="workshop-open-unit">of {full} {unit}</span>
      {error && <span className="workshop-open-error error-text" id={errorId} role="alert">{error}</span>}
    </span> : <>
      <button type="button" ref={button} className="workshop-open-amount" aria-disabled={pending || undefined}
        aria-label={shown ? `Change the amount left in the opened pack of ${item.name}, now ${shown}`
          : `Record the amount left in the opened pack of ${item.name}, a full pack is ${full} ${unit}`}
        onClick={() => { if (!pending) open(); }}>
        {shown ?? <span className="workshop-unknown">{full} {unit} pack</span>}
      </button>
      {percent !== null && <span className="workshop-gauge" role="img" aria-label={`${percent}% left`}>
        <i style={{ width: `${percent}%` }} />
      </span>}
    </>}
  </span>;
}

function cell(column: ColumnId, item: InventoryItem, currency: string, control: ReactNode): ReactNode {
  switch (column) {
    case 'category': return plain(item.category);
    case 'brand': return <Clipped text={item.brand} />;
    case 'part_number': return <Clipped text={item.part_number} />;
    case 'colour': return <Colour text={item.colour} />;
    case 'dimensions': return <Clipped text={item.dimensions} />;
    case 'notes': return <Clipped text={item.notes} />;
    case 'supplier': return item.supplier?.title.trim() ? <Clipped text={item.supplier.title} /> : <Missing label="None" />;
    case 'quantity':
    case 'open_amount': return control;
    case 'pack_quantity': return item.pack_quantity === null || item.pack_quantity === undefined
      ? <Missing label="Unknown" /> : item.pack_quantity.toLocaleString();
    case 'price': return estimated(item, money(item.price_cents ?? null, currency, formatMoney, 'No price'));
    case 'unit_price': {
      // A measured item is priced per unit of its measure, so the unit is part of the text: $0.03/g.
      const text = money(unitPriceCents(item), currency, formatUnitPrice, 'No price');
      return estimated(item, unitPriceCents(item) !== null && isMeasured(item) ? <>{text}/{item.measure_unit}</> : text);
    }
    case 'stock_value': return stockValueCents(item) === null ? <Missing label="Unknown" />
      : estimated(item, money(stockValueCents(item), currency, formatMoney, 'Unknown'));
  }
}

/** The least width of the Item column, in pixels. */
const itemMinWidth = 220;
/** The least width of each text column, in pixels. A table with many columns scrolls sideways instead of squeezing them. */
const textMinWidth: Partial<Record<ColumnId, number>> = {
  category: 120, colour: 140, dimensions: 150, brand: 120, part_number: 120, notes: 160, supplier: 140,
};

/** The width of a column other than Item, in pixels: the number columns are narrow and the text columns have a minimum. */
function pixelWidth(id: ColumnId): number {
  if (id === 'quantity') return 148;
  if (id === 'open_amount') return 170;
  return columnDefs[id].numeric ? 104 : textMinWidth[id] ?? 120;
}

/** Column widths: Item takes a share that shrinks as columns are added (never under 220 px); every other column has a fixed width. */
function widthOf(id: ColumnId | 'name', count: number): string {
  if (id === 'name') return count <= 2 ? '46%' : count <= 4 ? '38%' : '30%';
  return `${pixelWidth(id)}px`;
}

/** The table is at least as wide as its columns' minimums; a narrower window scrolls the table sideways. */
function tableMinWidth(columns: readonly ColumnId[]): number {
  return Math.max(540, itemMinWidth + columns.reduce((sum, id) => sum + pixelWidth(id), 0));
}

export default function WorkshopInventoryTable({ tableRef, items, columns, currency, sort, onSort, onOpen, onStep, stepping, onOpenAmount }: Props) {
  const all: Array<ColumnId | 'name'> = ['name', ...columns];
  const caption = captionFields(columns);
  return <div className="workshop-table">
    <table ref={tableRef} aria-label="Workshop inventory items"
      style={{ minWidth: tableMinWidth(columns) }}>
      <colgroup>{all.map(id => <col key={id} style={{ width: widthOf(id, all.length) }} />)}</colgroup>
      <thead><tr>{all.map(id => {
        const { label, numeric } = columnDefs[id];
        const active = sort.field === id;
        const Icon = active ? (sort.direction === 'asc' ? ArrowUp : ArrowDown) : ChevronsUpDown;
        return <th key={id} scope="col" className={`workshop-column-${id}${numeric ? ' workshop-numeric' : ''}`}
          aria-sort={active ? (sort.direction === 'asc' ? 'ascending' : 'descending') : undefined}>
          <button type="button" className="workshop-sort" onClick={() => onSort(id)}>
            {label}<Icon size={13} aria-hidden="true" />
          </button>
        </th>;
      })}</tr></thead>
      <tbody>{items.map(item => {
        const sub = caption.map(field => item[field]).filter(Boolean).join(' · ');
        return <tr key={item.id}>
          <th scope="row">
            <button type="button" className="workshop-item-name" data-item-id={item.id}
              aria-label={item.name} onClick={() => onOpen(item)}>
              <WorkshopThumbnail key={item.photo_key ?? ''} photoKey={item.photo_key} />
              <span className="workshop-item-caption">
                <span>{item.name}</span>
                {sub && <span className="workshop-item-sub">{sub}</span>}
              </span>
            </button>
          </th>
          {columns.map(id => <td key={id} className={columnDefs[id].numeric ? 'workshop-stock' : undefined}>
            {cell(id, item, currency, id === 'quantity' ? <Stepper item={item} pending={stepping.has(item.id)} onStep={onStep} />
              : id === 'open_amount' ? <OpenAmount item={item} pending={stepping.has(item.id)} onSave={onOpenAmount} /> : null)}
          </td>)}
        </tr>;
      })}</tbody>
    </table>
  </div>;
}
