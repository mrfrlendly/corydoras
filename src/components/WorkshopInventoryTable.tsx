import type { RefObject } from 'react';
import { ArrowDown, ArrowUp, ChevronsUpDown } from 'lucide-react';
import type { InventoryItem } from '../inventoryApi';
import WorkshopThumbnail from './WorkshopThumbnail';
import { listColumns, swatchColours, type ListSort, type SortField } from './workshopList';

interface Props {
  tableRef: RefObject<HTMLTableElement | null>;
  items: InventoryItem[];
  sort: ListSort;
  onSort: (field: SortField) => void;
  onOpen: (item: InventoryItem) => void;
}

function Colour({ text }: { text: string }) {
  if (!text) return <span className="workshop-unknown"><span aria-hidden="true">—</span><span className="sr-only">Unknown</span></span>;
  const colours = swatchColours(text);
  return <span className="workshop-colour">
    {colours.length > 0 && <span className="workshop-swatch" aria-hidden="true">
      {colours.map((colour, index) => <i key={index} style={{ backgroundColor: colour }} />)}
    </span>}
    {text}
  </span>;
}

export default function WorkshopInventoryTable({ tableRef, items, sort, onSort, onOpen }: Props) {
  return <div className="workshop-table">
    <table ref={tableRef} aria-label="Workshop inventory items">
      <thead><tr>{listColumns.map(({ field, label }) => {
        const active = sort.field === field;
        const Icon = active ? (sort.direction === 'asc' ? ArrowUp : ArrowDown) : ChevronsUpDown;
        return <th key={field} scope="col" className={`workshop-column-${field}`}
          aria-sort={active ? (sort.direction === 'asc' ? 'ascending' : 'descending') : undefined}>
          <button type="button" className="workshop-sort" onClick={() => onSort(field)}>
            {label}<Icon size={13} aria-hidden="true" />
          </button>
        </th>;
      })}</tr></thead>
      <tbody>{items.map(item => (
        <tr key={item.id}>
          <th scope="row">
            <button type="button" className="workshop-item-name" data-item-id={item.id}
              aria-label={item.name} onClick={() => onOpen(item)}>
              <WorkshopThumbnail key={item.photo_key ?? ''} photoKey={item.photo_key} />
              <span className="workshop-item-caption">
                <span>{item.name}</span>
                {(item.brand || item.part_number) && <span className="workshop-item-sub">
                  {[item.brand, item.part_number].filter(Boolean).join(' · ')}
                </span>}
              </span>
            </button>
          </th>
          <td>{item.category}</td>
          <td><Colour text={item.colour} /></td>
          <td className="workshop-stock">{item.quantity?.toLocaleString() ?? <span className="workshop-unknown">Unknown</span>}</td>
          <td className="workshop-stock">{item.pack_quantity?.toLocaleString() ?? <span className="workshop-unknown"><span aria-hidden="true">—</span><span className="sr-only">Unknown</span></span>}</td>
        </tr>
      ))}</tbody>
    </table>
  </div>;
}
