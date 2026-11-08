import { useEffect, useState } from 'react';
import { Check, X } from 'lucide-react';
import { api, errText, fmtDate, Language, Proposal, ProposalChange } from '../api';
import { useToast } from './Toast';

interface Props {
  language: Language;
  /** Open proposals from the status poll; a change reloads the list. */
  openCount: number;
  tick: number;
  /** A proposal became a new version. */
  onAccepted: (version: number) => void;
}

function short(v: unknown): string {
  if (v === null || v === undefined) return '—';
  const s = typeof v === 'string' ? v : JSON.stringify(v);
  return s.length > 160 ? `${s.slice(0, 157)}…` : s;
}

function ChangeRow({ c }: { c: ProposalChange }) {
  return (
    <li className="proposal-change">
      <span className={`proposal-op op-${c.op}`}>{c.op}</span>
      <code className="mono">{c.path}</code>
      <div className="proposal-values">
        {c.op !== 'add' && <span className="proposal-before mono">{short(c.before)}</span>}
        {c.op === 'replace' && <span aria-hidden> → </span>}
        {c.op !== 'remove' && <span className="proposal-after mono">{short(c.after)}</span>}
      </div>
    </li>
  );
}

export default function ProposalsPanel({ language, openCount, tick, onAccepted }: Props) {
  const toast = useToast();
  const [items, setItems] = useState<Proposal[]>([]);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    api
      .listProposals(language.id)
      .then(setItems)
      .catch((e) => toast.error(errText(e)));
  }, [language.id, openCount, tick, toast]);

  if (items.length === 0) return null;

  const decide = async (p: Proposal, accept: boolean) => {
    setBusy(p.id);
    try {
      if (accept) {
        const v = await api.acceptProposal(p.id);
        toast.info(`Accepted. Saved as version ${v}.`);
        onAccepted(v);
      } else {
        await api.rejectProposal(p.id);
        toast.info('Proposal rejected.');
      }
      setItems((list) => list.filter((x) => x.id !== p.id));
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="proposals" aria-label="Proposed amendments">
      <h3>
        {items.length} proposed amendment{items.length === 1 ? '' : 's'}
        <span className="muted small"> · from agents through the MCP server; nothing changes until you accept</span>
      </h3>
      {items.map((p) => (
        <article key={p.id} className="proposal">
          <header className="proposal-head">
            <div>
              <div className="proposal-summary">{p.summary}</div>
              <div className="hint">
                {p.author || 'An agent'} · {fmtDate(p.created_at)} · based on v{p.base_version}
              </div>
            </div>
            <div className="spacer" />
            <button className="btn small" onClick={() => decide(p, false)} disabled={busy !== null}>
              <X size={13} /> Reject
            </button>
            <button className="btn small primary" onClick={() => decide(p, true)} disabled={busy !== null || p.problem !== null}>
              <Check size={13} /> Accept as new version
            </button>
          </header>
          {p.rationale && <p className="proposal-rationale">{p.rationale}</p>}
          {!p.problem && p.new_notes.length > 0 && (
            <div className="proposal-notes">
              <b>Accepting adds {p.new_notes.length === 1 ? 'this review note' : 'these review notes'}:</b>
              <ul>
                {p.new_notes.map((n, i) => (
                  <li key={i}>{n}</li>
                ))}
              </ul>
            </div>
          )}
          {p.problem ? (
            <div className="error-box">This proposal no longer applies to the latest version: {p.problem}</div>
          ) : (
            <ul className="proposal-changes">
              {p.changes.map((c, i) => (
                <ChangeRow key={i} c={c} />
              ))}
            </ul>
          )}
        </article>
      ))}
    </section>
  );
}
