import { useEffect, useState } from 'react';
import { ChevronRight, ScanSearch } from 'lucide-react';
import { api, Coverage, errText, fmtDate, Gap, Language, UsageInsight } from '../api';
import { useToast } from './Toast';

interface Props {
  language: Language;
  tick: number;
  onBoard?: (idPrefix: string | null) => void;
  onDocument?: () => void;
  onActivity?: () => void;
}

/** Where the gap cards and the reference ids lead. */
interface Links {
  onBoard?: (idPrefix: string | null) => void;
  onDocument?: () => void;
  onActivity?: () => void;
}

const SEVERITY: { id: Gap['severity']; title: string; hint: string }[] = [
  { id: 'high', title: 'The language cannot answer this', hint: 'An agent gets no value or a guess.' },
  { id: 'medium', title: 'The answer is weak', hint: 'The value exists, but little backs it.' },
  { id: 'low', title: 'Worth a look', hint: '' },
];

const PERIODS = [
  { days: 30, label: 'Last 30 days' },
  { days: 90, label: 'Last 90 days' },
  { days: 365, label: 'Last year' },
];

const SHORT_ID = /^[0-9a-f]{8}$/;

/** A reference id. With a Board to go to, a button that shows the reference there. */
function RefId({ id, onBoard }: { id: string; onBoard?: (idPrefix: string | null) => void }) {
  if (!onBoard || !SHORT_ID.test(id)) return <code className="mono">{id}</code>;
  return (
    <button type="button" className="id-link mono" onClick={() => onBoard(id)} title="Show this reference on the Board">
      {id}
    </button>
  );
}

/** Text from the report with `ids` in backticks: each one becomes a code chip. */
function Coded({ text, onBoard }: { text: string; onBoard?: (idPrefix: string | null) => void }) {
  const parts = text.split('`');
  return <>{parts.map((p, i) => (i % 2 === 1 ? <RefId key={i} id={p} onBoard={onBoard} /> : p))}</>;
}

function plural(n: number, one: string, many: string) {
  return n === 1 ? one : many;
}

function Ids({ ids, onBoard }: { ids: string[]; onBoard?: (idPrefix: string | null) => void }) {
  if (ids.length === 0) return null;
  return (
    <span className="gap-ids">
      {ids.slice(0, 8).map((id) => (
        <RefId key={id} id={id} onBoard={onBoard} />
      ))}
      {ids.length > 8 && <span className="muted"> +{ids.length - 8}</span>}
    </span>
  );
}

/** Kinds of gap (crates/corydoras-core/src/coverage.rs) whose fix starts on the Board. */
const BOARD_GAPS = [
  'weak-measure',
  'contradiction',
  'thin-family',
  'missing-views',
  'no-measures',
  'no-rules',
  'no-anti-reference',
  'no-colours',
  'unused-reference',
];

/** The one place to go for a gap, or nothing. */
function GapAction({ g, links }: { g: Gap; links: Links }) {
  const { onBoard, onDocument, onActivity } = links;
  let label = '';
  let run: (() => void) | undefined;
  // A failed analysis has its Retry on the card; references that wait are in Activity.
  if ((BOARD_GAPS.includes(g.kind) || (g.kind === 'not-analyzed' && g.evidence.length > 0)) && onBoard) {
    label = g.evidence.length > 0 ? 'Show on the Board' : 'Open the Board';
    run = () => onBoard(g.evidence[0] ?? null);
  } else if ((g.kind === 'review-notes' || g.kind === 'no-document') && onDocument) {
    label = 'Open the document';
    run = onDocument;
  } else if (g.kind === 'not-analyzed' && onActivity) {
    label = 'Open Activity';
    run = onActivity;
  }
  if (!run) return null;
  return (
    <>
      {' '}
      <button type="button" className="link-btn small gap-action" onClick={run}>
        {label}
      </button>
    </>
  );
}

/** How many medium gaps show before "Show more". */
const MEDIUM_FIRST = 3;

function CoverageSection({ c, links }: { c: Coverage; links: Links }) {
  const [family, setFamily] = useState('');
  // The low gaps are many and minor: they start closed.
  const [open, setOpen] = useState<Record<Gap['severity'], boolean>>({ high: true, medium: true, low: false });
  const [allMedium, setAllMedium] = useState(false);
  const gaps = c.gaps.filter((g) => !family || g.family === family);
  return (
    <section className="panel">
      <div className="row" style={{ marginTop: 0 }}>
        <h3 className="grow" style={{ marginBottom: 0 }}>
          Coverage{c.version ? ` of v${c.version}` : ''}
        </h3>
        {c.families.length > 1 && (
          <select className="select small" value={family} onChange={(e) => setFamily(e.target.value)} aria-label="Family">
            <option value="">All families</option>
            {c.families.map((f) => (
              <option key={f.id} value={f.id}>
                {f.name}
              </option>
            ))}
          </select>
        )}
      </div>
      <p className="muted small">
        What this language cannot answer yet, worked out from the latest version and the analyses of the references. It needs no model call. Agents
        get the same report with <code className="mono">get_coverage</code>.
      </p>

      {c.families.length > 0 && (
        <table className="vtable">
          <thead>
            <tr>
              <th>Family</th>
              <th>References</th>
              <th>Not this</th>
              <th>Measures</th>
              <th>Rules</th>
              <th>Views shown</th>
              <th>Views missing</th>
            </tr>
          </thead>
          <tbody>
            {c.families.map((f) => (
              <tr key={f.id}>
                <td>
                  {f.name} <span className="mono small muted">{f.id}</span>
                </td>
                <td className={f.references <= 1 ? 'cell-bad' : ''}>{f.references}</td>
                <td>{f.anti_references}</td>
                <td className={f.measures === 0 ? 'cell-bad' : ''}>{f.measures}</td>
                <td>{f.rules}</td>
                <td className="small">{f.views.join(', ') || '—'}</td>
                <td className={`small ${f.missing_views.length > 0 ? 'cell-warn' : ''}`}>{f.missing_views.join(', ') || '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {gaps.length === 0 ? (
        <div className="notice">{c.gaps.length === 0 ? 'No gaps found: every family has several references, the wanted views, and measures with evidence.' : 'No gaps in this family.'}</div>
      ) : (
        SEVERITY.map((s) => {
          const list = gaps.filter((g) => g.severity === s.id);
          if (list.length === 0) return null;
          const isOpen = open[s.id];
          const shown = s.id === 'medium' && !allMedium ? list.slice(0, MEDIUM_FIRST) : list;
          const listId = `gaps-${s.id}`;
          return (
            <div key={s.id} className={`gap-group gap-${s.id}`}>
              <h4>
                <button
                  type="button"
                  className="gap-toggle"
                  aria-expanded={isOpen}
                  aria-controls={listId}
                  onClick={() => setOpen((o) => ({ ...o, [s.id]: !o[s.id] }))}
                >
                  <ChevronRight size={13} className="gap-chevron" aria-hidden />
                  {s.title} <span className="tab-badge">{list.length}</span>
                </button>
                {s.hint && <span className="muted small"> · {s.hint}</span>}
              </h4>
              {isOpen && (
                <ul className="gaps" id={listId}>
                  {shown.map((g, i) => {
                    // The ids of a stale-evidence gap are no longer on the Board.
                    const onBoard = g.kind === 'stale-evidence' ? undefined : links.onBoard;
                    return (
                      <li key={i} className="gap">
                        <div className="gap-title">{g.title}</div>
                        <div className="small">
                          <Coded text={g.detail} onBoard={onBoard} /> {!g.detail.includes('`') && <Ids ids={g.evidence} onBoard={onBoard} />}
                        </div>
                        <div className="small gap-suggestion">
                          {g.suggestion}
                          <GapAction g={g} links={links} />
                        </div>
                      </li>
                    );
                  })}
                </ul>
              )}
              {isOpen && shown.length < list.length && (
                <button type="button" className="link-btn small gap-more" onClick={() => setAllMedium(true)}>
                  Show {list.length - shown.length} more
                </button>
              )}
            </div>
          );
        })
      )}
    </section>
  );
}

function UsageSection({
  u,
  days,
  onDays,
  onBoard,
}: {
  u: UsageInsight;
  days: number;
  onDays: (d: number) => void;
  onBoard?: (idPrefix: string | null) => void;
}) {
  return (
    <section className="panel">
      <div className="row" style={{ marginTop: 0 }}>
        <h3 className="grow" style={{ marginBottom: 0 }}>
          What agents use
        </h3>
        <select className="select small" value={days} onChange={(e) => onDays(Number(e.target.value))} aria-label="Period">
          {PERIODS.map((p) => (
            <option key={p.days} value={p.days}>
              {p.label}
            </option>
          ))}
        </select>
      </div>
      <p className="muted small">
        The MCP server notes each call that reads or checks this language. The notes stay in the library on this Mac. The server starts to note calls
        with this version, so earlier use is not counted.
      </p>

      {u.calls === 0 ? (
        <div className="notice">No agent has read this language since {u.since}.</div>
      ) : (
        <>
          <div className="stat-row">
            <div className="stat">
              <b>{u.calls}</b> {plural(u.calls, 'call', 'calls')}
            </div>
            <div className="stat" title="Calls that read the whole document: every family goes with it">
              <b>{u.document_reads}</b> whole-document {plural(u.document_reads, 'read', 'reads')}
            </div>
            <div className="stat">
              <b>{u.briefs}</b> project {plural(u.briefs, 'brief', 'briefs')}
            </div>
            <div className="stat">
              last call <b>{u.last_call ? fmtDate(u.last_call) : '—'}</b>
            </div>
          </div>
          <div className="small muted">
            {u.clients.map(([name, n]) => `${name} (${n})`).join(' · ')} · {u.tools.map(([name, n]) => `${name} ${n}`).join(' · ')}
          </div>

          {u.families.length > 0 && (
            <table className="vtable">
              <thead>
                <tr>
                  <th>Family</th>
                  <th title="Calls that asked for this family by name">Asked for</th>
                  <th title="Conformance and geometry checks for this family">Checked</th>
                  <th>Last use</th>
                </tr>
              </thead>
              <tbody>
                {u.families.map((f) => (
                  <tr key={f.id}>
                    <td>
                      {f.name}
                      {f.fetches === 0 && f.checks === 0 && <span className="tag-pill">not used</span>}
                    </td>
                    <td>{f.fetches}</td>
                    <td>{f.checks}</td>
                    <td className="small muted">{f.last_used ? fmtDate(f.last_used) : '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {u.unused_families.length > 0 && (
            <p className="small">
              <b>Not used:</b> {u.unused_families.join(', ')}. No call asked for {u.unused_families.length === 1 ? 'this family' : 'these families'} by name
              {u.document_reads + u.briefs > 0 ? '; an agent can still have read them in the whole document' : ''}.
            </p>
          )}
          {u.measures.length > 0 && (
            <p className="small">
              <b>Measures that agents checked:</b> {u.measures.map((m) => `${m.name} (${m.checks})`).join(', ')}.
            </p>
          )}
          {u.unchecked_measures.length > 0 && (
            <p className="small muted">
              <b>Never checked:</b> {u.unchecked_measures.join(', ')}.
            </p>
          )}
          {u.references.length > 0 && (
            <p className="small">
              <b>References that agents asked to see:</b>{' '}
              {u.references.map(([id, n]) => (
                <span key={id}>
                  <RefId id={id} onBoard={onBoard} /> ({n}){' '}
                </span>
              ))}
            </p>
          )}
        </>
      )}

      <h4 className="sub-head">Proposals that come back</h4>
      {u.recurring.length === 0 ? (
        <p className="muted small">No place in the document got more than one amendment proposal.</p>
      ) : (
        <ul className="gaps">
          {u.recurring.map((r) => (
            <li key={r.path} className="gap">
              <div className="gap-title mono">{r.path}</div>
              <div className="small">
                {r.proposals} proposals: {r.accepted} accepted, {r.rejected} rejected, {r.pending} open.
                {r.rejected >= 2 && r.accepted === 0 ? ' Agents keep asking for a change here that you reject: a note on a reference or a rule can say why the value stays.' : ''}
              </div>
              <div className="small muted">{r.summaries.join(' · ')}</div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export default function InsightView({ language, tick, onBoard, onDocument, onActivity }: Props) {
  const toast = useToast();
  const [coverage, setCoverage] = useState<Coverage | null>(null);
  const [usage, setUsage] = useState<UsageInsight | null>(null);
  const [days, setDays] = useState(90);

  useEffect(() => {
    let alive = true;
    api
      .coverage(language.id)
      .then((c) => alive && setCoverage(c))
      .catch((e) => toast.error(errText(e)));
    return () => {
      alive = false;
    };
  }, [language.id, tick, toast]);

  useEffect(() => {
    let alive = true;
    api
      .usageInsight(language.id, days)
      .then((u) => alive && setUsage(u))
      .catch((e) => toast.error(errText(e)));
    return () => {
      alive = false;
    };
  }, [language.id, days, tick, toast]);

  if (!coverage && !usage) {
    return (
      <div className="board-empty">
        <ScanSearch size={34} strokeWidth={1.3} className="drop-glyph" />
      </div>
    );
  }
  return (
    <div className="insight">
      {coverage && <CoverageSection c={coverage} links={{ onBoard, onDocument, onActivity }} />}
      {usage && <UsageSection u={usage} days={days} onDays={setDays} onBoard={onBoard} />}
    </div>
  );
}
