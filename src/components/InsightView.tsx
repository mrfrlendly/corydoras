import { useEffect, useState } from 'react';
import { ScanSearch } from 'lucide-react';
import { api, Coverage, errText, fmtDate, Gap, Language, UsageInsight } from '../api';
import { useToast } from './Toast';

interface Props {
  language: Language;
  tick: number;
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

/** Text from the report with `ids` in backticks: each one becomes a code chip. */
function Coded({ text }: { text: string }) {
  const parts = text.split('`');
  return (
    <>
      {parts.map((p, i) =>
        i % 2 === 1 ? (
          <code key={i} className="mono">
            {p}
          </code>
        ) : (
          p
        ),
      )}
    </>
  );
}

function plural(n: number, one: string, many: string) {
  return n === 1 ? one : many;
}

function Ids({ ids }: { ids: string[] }) {
  if (ids.length === 0) return null;
  return (
    <span className="gap-ids">
      {ids.slice(0, 8).map((id) => (
        <code key={id} className="mono">
          {id}
        </code>
      ))}
      {ids.length > 8 && <span className="muted"> +{ids.length - 8}</span>}
    </span>
  );
}

function CoverageSection({ c }: { c: Coverage }) {
  const [family, setFamily] = useState('');
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
          return (
            <div key={s.id} className={`gap-group gap-${s.id}`}>
              <h4>
                {s.title} <span className="tab-badge">{list.length}</span>
                {s.hint && <span className="muted small"> · {s.hint}</span>}
              </h4>
              <ul className="gaps">
                {list.map((g, i) => (
                  <li key={i} className="gap">
                    <div className="gap-title">{g.title}</div>
                    <div className="small">
                      <Coded text={g.detail} /> {!g.detail.includes('`') && <Ids ids={g.evidence} />}
                    </div>
                    <div className="small gap-suggestion">{g.suggestion}</div>
                  </li>
                ))}
              </ul>
            </div>
          );
        })
      )}
    </section>
  );
}

function UsageSection({ u, days, onDays }: { u: UsageInsight; days: number; onDays: (d: number) => void }) {
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
                  <code className="mono">{id}</code> ({n}){' '}
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

export default function InsightView({ language, tick }: Props) {
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
      {coverage && <CoverageSection c={coverage} />}
      {usage && <UsageSection u={usage} days={days} onDays={setDays} />}
    </div>
  );
}
