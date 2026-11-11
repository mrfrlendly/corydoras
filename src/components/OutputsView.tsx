import { Fragment, useEffect, useRef, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { Check, ClipboardCheck, Loader, Printer, Ruler, Trash2, Undo2, X } from 'lucide-react';
import { api, ConformanceFinding, errText, fileUrl, fmtDate, GeometryReport, Language, MODEL_EXTENSIONS, Output, PRINT_OUTCOMES, PrintOutcome, Spot } from '../api';
import { useToast } from './Toast';

const UP_AXES = ['+z', '-z', '+y', '-y', '+x', '-x'];

/** A measured length or angle: two decimals are enough. */
function geo(v: number, unit: string) {
  return `${Math.round(v * 100) / 100} ${unit}`;
}

function at(s: Spot) {
  return `near x ${s.at[0]}, y ${s.at[1]}, z ${s.at[2]}`;
}

/** The measured values of a geometry check, then the verdict of each. */
function GeometryBlock({ g }: { g: GeometryReport }) {
  const [open, setOpen] = useState(false);
  const c = g.check;
  return (
    <div className="output-check">
      <button className="link-btn small" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        Geometry of {g.file}: {c ? `${c.pass} pass · ${c.fail} fail · ${c.warn} warn · ${c.unknown} unknown` : 'measured'} {open ? '▾' : '▸'}
      </button>
      {open && (
        <>
          <div className="small muted">
            {g.format.toUpperCase()} · build direction {g.up} · {g.triangles.toLocaleString()} triangles · {g.bodies} bod{g.bodies === 1 ? 'y' : 'ies'} ·{' '}
            {g.size.map((v) => Math.round(v * 10) / 10).join(' × ')} mm{g.volume_cm3 !== null ? ` · ${Math.round(g.volume_cm3 * 10) / 10} cm³` : ''}
          </div>
          <ul className="geo-list small">
            {g.min_wall && (
              <li>
                Thinnest wall <strong>{geo(g.min_wall.value, 'mm')}</strong> {at(g.min_wall)}
                {g.median_wall !== null ? ` (typical wall ${geo(g.median_wall, 'mm')})` : ''}
              </li>
            )}
            {g.max_overhang && (
              <li>
                {g.max_overhang.value <= 0 ? (
                  'No overhang above the first layers'
                ) : (
                  <>
                    Steepest overhang <strong>{geo(g.max_overhang.value, '°')}</strong> from the vertical {at(g.max_overhang)}
                  </>
                )}
              </li>
            )}
            {g.flat_overhang && (
              <li>
                A flat ceiling reaches <strong>{geo(g.flat_overhang.value, 'mm')}</strong> past its support {at(g.flat_overhang)}
              </li>
            )}
            {g.max_bridge && (
              <li>
                Longest bridge <strong>{geo(g.max_bridge.value, 'mm')}</strong> {at(g.max_bridge)}
              </li>
            )}
            {g.min_clearance && (
              <li>
                Smallest gap between bodies <strong>{geo(g.min_clearance.value, 'mm')}</strong> {at(g.min_clearance)}
              </li>
            )}
            {g.notes.map((n, i) => (
              <li key={i} className="muted">
                {n}
              </li>
            ))}
          </ul>
          {c && (
            <>
              <ul className="findings">
                {c.findings.map((f, i) => (
                  <FindingRow key={i} f={f} />
                ))}
              </ul>
              <div className="small muted">
                {c.rule_sets && c.rule_sets.length > 0 ? `Hard rules from ${c.rule_sets.join(', ')}.` : 'No hard rule set applied, so no hard rule judged anything.'}
              </div>
            </>
          )}
        </>
      )}
    </div>
  );
}

interface Props {
  language: Language;
  tick: number;
  /** Outputs without a verdict, from the status poll; a change reloads the list. */
  pending: number;
  onChanged: () => void;
}

const VERDICT_LABEL: Record<Output['verdict'], string> = { pending: 'Waiting', accepted: 'Accepted', rejected: 'Rejected' };

function fmtNum(v: number, unit: string | null) {
  const n = Number.isInteger(v) ? String(v) : String(Math.round(v * 1000) / 1000);
  return unit ? `${n} ${unit}` : n;
}

function FindingRow({ f }: { f: ConformanceFinding }) {
  return (
    <li className={`finding finding-${f.verdict}`}>
      <span className="finding-verdict">{f.verdict}</span>
      <span className="finding-body">
        <strong>{f.measure}</strong> {fmtNum(f.value, f.unit)}
        {f.expected && <span className="muted"> · expected {f.expected}</span>}
        {f.detail && <span className="muted"> · {f.detail}</span>}
      </span>
    </li>
  );
}

function OutputCard({
  o,
  busy,
  onDecide,
  onRemove,
  onUpdated,
}: {
  o: Output;
  busy: boolean;
  onDecide: (v: Output['verdict'], note: string) => void;
  onRemove: () => void;
  onUpdated: (next: Output) => void;
}) {
  const toast = useToast();
  const [note, setNote] = useState(o.verdict_note);
  const [open, setOpen] = useState(false);
  const [up, setUp] = useState('+z');
  const [checking, setChecking] = useState(false);
  const [printing, setPrinting] = useState(false);
  const [outcome, setOutcome] = useState<PrintOutcome>('fine');
  const [material, setMaterial] = useState('');
  const [printNote, setPrintNote] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);
  const deleteRef = useRef<HTMLButtonElement>(null);
  const refocusDelete = useRef(false);
  useEffect(() => setNote(o.verdict_note), [o.verdict_note]);
  const c = o.check;

  // Cancel puts focus back on the delete button.
  useEffect(() => {
    if (!confirmDelete && refocusDelete.current) {
      refocusDelete.current = false;
      deleteRef.current?.focus();
    }
  }, [confirmDelete]);

  const cancelDelete = () => {
    refocusDelete.current = true;
    setConfirmDelete(false);
  };

  const checkModel = async () => {
    const picked = await openDialog({
      multiple: false,
      title: `Choose the model of “${o.name}”`,
      filters: [{ name: '3D models', extensions: MODEL_EXTENSIONS }],
    });
    if (typeof picked !== 'string') return;
    setChecking(true);
    try {
      const next = await api.checkOutputGeometry(o.id, picked, up);
      onUpdated(next);
      const r = next.geometry?.check;
      toast.info(r ? `Geometry checked: ${r.pass} pass, ${r.fail} fail, ${r.warn} warn, ${r.unknown} unknown.` : 'Geometry checked.');
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setChecking(false);
    }
  };

  const logPrint = async () => {
    if (!material.trim()) {
      toast.error('Say which material the part was printed in.');
      return;
    }
    try {
      await api.addPrintResult({
        result: outcome,
        material,
        rule: '',
        rule_set: null,
        value: null,
        unit: null,
        note: printNote || `Print of “${o.name}”.`,
        language: null,
        output: o.id,
      });
      toast.info('Saved in the print log. Agents get it with the hard rules.');
      setPrinting(false);
      setPrintNote('');
    } catch (e) {
      toast.error(errText(e));
    }
  };
  return (
    <article className={`output output-${o.verdict}`}>
      <a href={fileUrl(o.file_rel)} target="_blank" rel="noreferrer" className="output-thumb" title="Open the full image">
        <img src={fileUrl(o.thumb_rel)} alt={o.name} loading="lazy" />
      </a>
      <div className="output-main">
        <div className="output-head">
          <span className="output-name">{o.name}</span>
          <span className={`output-state state-${o.verdict}`}>{VERDICT_LABEL[o.verdict]}</span>
        </div>
        <div className="small muted">
          {o.author || 'An agent'}
          {o.language_version ? ` · from v${o.language_version}` : ''}
          {o.family ? ` · ${o.family}` : ''}
          {o.project ? ` · ${o.project}` : ''} · {fmtDate(o.created_at)}
        </div>
        {o.note && <p className="output-note">{o.note}</p>}

        {c && (
          <div className="output-check">
            <button className="link-btn small" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
              Check: {c.pass} pass · {c.fail} fail · {c.warn} warn · {c.unknown} unknown {open ? '▾' : '▸'}
            </button>
            {open && (
              <>
                <ul className="findings">
                  {c.findings.map((f, i) => (
                    <FindingRow key={i} f={f} />
                  ))}
                </ul>
                <div className="small muted">
                  {c.rule_sets && c.rule_sets.length > 0 ? `Hard rules from ${c.rule_sets.join(', ')}.` : 'No hard rule set applied, so no hard rule judged anything.'}
                </div>
              </>
            )}
          </div>
        )}
        {!c && o.measurements.length > 0 && (
          <div className="small muted">Measurements: {o.measurements.map((m) => `${m.name} ${fmtNum(m.value, m.unit)}`).join(', ')} (not checked)</div>
        )}
        {o.geometry && <GeometryBlock g={o.geometry} />}

        <div className="output-tools">
          <button className="btn small" disabled={busy || checking} onClick={checkModel} title="Measure an STL, 3MF, OBJ or STEP file of this design against the hard rules">
            {checking ? <Loader size={13} className="spin" /> : <Ruler size={13} />} {o.geometry ? 'Check a model again…' : 'Check a model…'}
          </button>
          <label className="small muted up-pick" title="The axis that points up on the printer. Overhangs and bridges depend on it.">
            Build direction
            <select className="select small" value={up} onChange={(e) => setUp(e.target.value)} disabled={checking}>
              {UP_AXES.map((a) => (
                <option key={a} value={a}>
                  {a}
                </option>
              ))}
            </select>
          </label>
          <button className="btn small" disabled={busy} onClick={() => setPrinting((v) => !v)} aria-expanded={printing}>
            <Printer size={13} /> Log a print…
          </button>
        </div>
        {printing && (
          <div className="print-form">
            <select className="select small" value={outcome} onChange={(e) => setOutcome(e.target.value as PrintOutcome)} aria-label="Result of the print">
              {PRINT_OUTCOMES.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
            </select>
            <input className="small-input" placeholder="Material, for example PETG clear" value={material} onChange={(e) => setMaterial(e.target.value)} maxLength={120} />
            <input className="output-note-input" placeholder="What you saw (optional)" value={printNote} onChange={(e) => setPrintNote(e.target.value)} maxLength={400} />
            <button className="btn small primary" onClick={logPrint}>
              Save
            </button>
            <button className="btn small" onClick={() => setPrinting(false)}>
              Cancel
            </button>
          </div>
        )}

        <div
          className="output-actions"
          onKeyDown={(e) => {
            if (confirmDelete && e.key === 'Escape') {
              e.preventDefault();
              cancelDelete();
            }
          }}
        >
          {/* Keys: React must not reuse the buttons of one row for the other, or autoFocus does not run. */}
          {confirmDelete ? (
            <Fragment key="confirm">
              <button type="button" className="btn small danger" disabled={busy} onClick={onRemove}>
                <Trash2 size={13} /> Delete output
              </button>
              <button type="button" className="btn small" autoFocus disabled={busy} onClick={cancelDelete}>
                Cancel
              </button>
              <span className="muted small">{o.item_id ? 'The render and the reference that it made are deleted.' : 'The render is deleted.'}</span>
            </Fragment>
          ) : (
            <Fragment key="actions">
              <input
                className="output-note-input"
                placeholder={o.verdict === 'rejected' ? 'Why not (it goes to the language)' : 'Note for the language (optional)'}
                value={note}
                maxLength={400}
                onChange={(e) => setNote(e.target.value)}
                disabled={busy}
              />
              <button className="btn small primary" disabled={busy || o.verdict === 'accepted'} onClick={() => onDecide('accepted', note)}>
                <Check size={13} /> Accept
              </button>
              <button className="btn small" disabled={busy || o.verdict === 'rejected'} onClick={() => onDecide('rejected', note)}>
                <X size={13} /> Reject
              </button>
              {o.verdict !== 'pending' && (
                <button className="btn small" disabled={busy} onClick={() => onDecide('pending', '')} title="Remove the verdict and the reference it made">
                  <Undo2 size={13} /> Undo
                </button>
              )}
              <button
                ref={deleteRef}
                type="button"
                className="btn small"
                disabled={busy}
                onClick={() => setConfirmDelete(true)}
                title="Delete this output"
                aria-label="Delete this output"
              >
                <Trash2 size={13} />
              </button>
            </Fragment>
          )}
        </div>
      </div>
    </article>
  );
}

export default function OutputsView({ language, tick, pending, onChanged }: Props) {
  const toast = useToast();
  const [items, setItems] = useState<Output[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = () =>
    api
      .listOutputs(language.id)
      .then(setItems)
      .catch((e) => toast.error(errText(e)));

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language.id, tick, pending]);

  const decide = async (o: Output, verdict: Output['verdict'], note: string) => {
    setBusy(o.id);
    try {
      const next = await api.setOutputVerdict(o.id, verdict, note);
      setItems((list) => (list ?? []).map((x) => (x.id === o.id ? next : x)));
      toast.info(
        verdict === 'accepted'
          ? 'Accepted. It now supports the language as a reference.'
          : verdict === 'rejected'
            ? 'Rejected. It is an anti-reference now; the next version avoids it.'
            : 'Verdict removed.',
      );
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setBusy(null);
    }
  };

  // The card asks first (Delete output / Cancel), so this deletes at once.
  const remove = async (o: Output) => {
    setBusy(o.id);
    try {
      await api.removeOutput(o.id);
      setItems((list) => (list ?? []).filter((x) => x.id !== o.id));
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setBusy(null);
    }
  };

  if (items === null) return null;
  if (items.length === 0) {
    return (
      <div className="board-empty">
        <div className="drop-target">
          <ClipboardCheck size={34} strokeWidth={1.3} className="drop-glyph" />
          <div className="drop-big">No outputs yet</div>
          <p>
            When an agent makes a design from this language, it sends a render here with <code className="mono">submit_output</code>. Accept it and it
            supports the language; reject it and it becomes an anti-reference that the next version avoids.
          </p>
        </div>
      </div>
    );
  }
  return (
    <div className="outputs">
      {items.map((o) => (
        <OutputCard
          key={o.id}
          o={o}
          busy={busy === o.id}
          onDecide={(v, n) => decide(o, v, n)}
          onRemove={() => remove(o)}
          onUpdated={(next) => setItems((list) => (list ?? []).map((x) => (x.id === next.id ? next : x)))}
        />
      ))}
    </div>
  );
}
