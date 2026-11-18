import { Plus, X } from 'lucide-react';
import type { Spec, SpecSource } from '../inventoryApi';
import { addSpec, editSpec, removeSpec } from './workshopForm';

const roles: [string, string][] = [
  ['thread', 'Thread'], ['pitch', 'Pitch'], ['length', 'Length'], ['outer_diameter', 'Outer diameter'],
  ['hole_diameter', 'Insert hole diameter'], ['clearance_hole', 'Clearance hole'], ['tap_drill', 'Tap drill'],
  ['diameter', 'Diameter'], ['net_weight', 'Net weight'], ['material', 'Material'],
  ['nozzle_temp', 'Nozzle temperature'], ['bed_temp', 'Bed temperature'], ['drying', 'Drying'], ['other', 'Other'],
];
const units = ['', 'mm', 'g', 'kg', '°C'];

export const sourceLabel: Record<SpecSource, string> = {
  label: 'Printed',
  notation: 'Printed, role read from notation',
  standard: 'Standard, not measured',
  entered: 'Entered',
};

interface Props {
  specs: Spec[];
  onChange: (specs: Spec[]) => void;
  disabled: boolean;
  /** Names the table for assistive technology, for example the item name. */
  label: string;
}

/** The structured dimensions of one item. Editing a value makes it an entered one; the printed evidence stays. */
export default function WorkshopSpecsTable({ specs, onChange, disabled, label }: Props) {
  return <div className="workshop-specs">
    {specs.length > 0 && <table>
      <caption className="sr-only">Specs of {label}</caption>
      <thead><tr><th>Role</th><th>Value</th><th>Unit</th><th>Tolerance</th><th>Source</th><th><span className="sr-only">Remove</span></th></tr></thead>
      <tbody>{specs.map((spec, i) => <tr key={i}>
        <td>
          <select className="select" aria-label={`Spec ${i + 1} role`} disabled={disabled} value={spec.role}
            onChange={e => onChange(editSpec(specs, i, { role: e.target.value }))}>
            {roles.map(([value, title]) => <option key={value} value={value}>{title}</option>)}
          </select>
          {spec.role === 'other' && <input aria-label={`Spec ${i + 1} name`} placeholder="Name" maxLength={60} disabled={disabled}
            value={spec.name} onChange={e => onChange(editSpec(specs, i, { name: e.target.value }))} />}
        </td>
        <td><input aria-label={`Spec ${i + 1} value`} maxLength={120} disabled={disabled} value={spec.value}
          onChange={e => onChange(editSpec(specs, i, { value: e.target.value }))} /></td>
        <td><select className="select" aria-label={`Spec ${i + 1} unit`} disabled={disabled} value={spec.unit}
          onChange={e => onChange(editSpec(specs, i, { unit: e.target.value }))}>
          {units.map(unit => <option key={unit} value={unit}>{unit || '—'}</option>)}
        </select></td>
        <td><input aria-label={`Spec ${i + 1} tolerance`} maxLength={60} disabled={disabled} value={spec.tolerance}
          onChange={e => onChange(editSpec(specs, i, { tolerance: e.target.value }))} /></td>
        <td>
          <span className={`workshop-source-badge ${spec.source}`}>{sourceLabel[spec.source]}</span>
          {spec.evidence && <span className="small muted selectable workshop-evidence">“{spec.evidence}”</span>}
        </td>
        <td><button type="button" className="link-btn small" disabled={disabled} aria-label={`Remove spec ${i + 1}`}
          onClick={() => onChange(removeSpec(specs, i))}><X size={13} aria-hidden="true" /></button></td>
      </tr>)}</tbody>
    </table>}
    <button type="button" className="btn small" disabled={disabled} onClick={() => onChange(addSpec(specs))}>
      <Plus size={13} aria-hidden="true" /> Add spec
    </button>
  </div>;
}
