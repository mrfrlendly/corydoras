import { useEffect, useState } from 'react';
import { Check, ClipboardCheck, Trash2, Undo2, X } from 'lucide-react';
import { api, ConformanceFinding, errText, fileUrl, fmtDate, Language, Output } from '../api';
import { useToast } from './Toast';

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

function OutputCard({ o, busy, onDecide, onRemove }: { o: Output; busy: boolean; onDecide: (v: Output['verdict'], note: string) => void; onRemove: () => void }) {
  const [note, setNote] = useState(o.verdict_note);
  const [open, setOpen] = useState(false);
  useEffect(() => setNote(o.verdict_note), [o.verdict_note]);
  const c = o.check;
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

        <div className="output-actions">
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
          <button className="btn small" disabled={busy} onClick={onRemove} title="Delete this output" aria-label="Delete this output">
            <Trash2 size={13} />
          </button>
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

  const remove = async (o: Output) => {
    if (!window.confirm(`Delete "${o.name}"?${o.item_id ? ' The reference that it made is removed too.' : ''}`)) return;
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
        <OutputCard key={o.id} o={o} busy={busy === o.id} onDecide={(v, n) => decide(o, v, n)} onRemove={() => remove(o)} />
      ))}
    </div>
  );
}
