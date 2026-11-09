import { useCallback, useEffect, useState } from 'react';
import { ActivityReport, api, CurrentJob, errText, fmtDuration, fmtTokens, JobView, Language, LogEntry, streamLabel } from '../api';
import { CircleStop, Pause, Play, X } from 'lucide-react';
import { useToast } from './Toast';

interface Props {
  languages: Language[];
  onChanged: () => void;
  onOpenLanguage: (id: string) => void;
}

const ANALYSIS_STEPS = ['Prepare the image', 'Model reads the image', 'Save the analysis'];
const SOURCE_STEPS = ['Prepare the text', 'Model reads the source', 'Save the notes'];
const SYNTH_STEPS = ['Collect the analyses', 'Model writes the spec', 'Check the answer', 'Save the version'];

function stepIndex(c: CurrentJob): number {
  const p = c.phase.toLowerCase();
  if (c.kind === 'synthesize') {
    if (p.startsWith('collecting') || p.startsWith('starting')) return 0;
    if (p.startsWith('checking')) return 2;
    if (p.startsWith('saving')) return 3;
    return 1;
  }
  if (p.startsWith('preparing') || p.startsWith('starting')) return 0;
  if (p.startsWith('saving')) return 2;
  return 1;
}

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit', second: '2-digit' });
}

function NowCard({ c, now, onCancel }: { c: CurrentJob; now: number; onCancel: () => void }) {
  const steps = c.kind === 'synthesize' ? SYNTH_STEPS : c.kind === 'analyze_source' ? SOURCE_STEPS : ANALYSIS_STEPS;
  const at = stepIndex(c);
  const outTokens = Math.round((c.reasoning_chars + c.output_chars) / 4);
  const pct = c.max_tokens > 0 ? Math.min(100, (outTokens / c.max_tokens) * 100) : 0;
  const idleMs = c.last_data_ms ? now - c.last_data_ms : c.call_started_ms ? now - c.call_started_ms : 0;
  const inCall = at === 1 && c.call_started_ms !== null && c.stream !== '';
  const quiet = inCall && idleMs > 30_000;

  return (
    <section className="panel now-card">
      <div className="now-head">
        <div className="grow">
          <div className="eyebrow">
            {c.language_name} · {c.kind === 'synthesize' ? 'Language document' : c.kind === 'analyze_source' ? 'Source notes' : 'Image analysis'}
          </div>
          <h2>{c.title}</h2>
        </div>
        <button className="btn small ghost-danger" onClick={onCancel} disabled={c.cancel_requested}>
          <CircleStop size={13} />
          {c.cancel_requested ? 'Stopping…' : 'Cancel this job'}
        </button>
      </div>

      <ol className="steps">
        {steps.map((s, i) => (
          <li key={s} className={i < at ? 'done' : i === at ? 'active' : ''}>
            <span className="step-dot" />
            <span>
              {s}
              {i === 1 && c.max_attempts > 1 && c.attempt > 0 ? ` · attempt ${c.attempt} of ${c.max_attempts}` : ''}
            </span>
          </li>
        ))}
      </ol>

      <div className="now-line">
        <span className={`dot ${c.cancel_requested ? 'status-error' : 'dot-busy'}`} />
        {c.cancel_requested ? 'Stopping the job' : inCall ? streamLabel(c) : c.phase}
        {inCall && c.phase !== 'Waiting for the model' && !c.cancel_requested ? <span className="muted"> · {c.phase}</span> : null}
      </div>

      {inCall && (
        <>
          <div className="meter" title="Output tokens so far, compared with the limit for this call">
            <div className={`meter-fill ${c.stream === 'thinking' ? 'thinking' : ''}`} style={{ transform: `translateX(${Math.max(pct, 1.5) - 100}%)` }} />
          </div>
          <div className="metrics">
            <div>
              <span>Output so far</span>~{fmtTokens(outTokens)} tokens
            </div>
            <div>
              <span>Thinking / answer</span>~{fmtTokens(Math.round(c.reasoning_chars / 4))} / ~{fmtTokens(Math.round(c.output_chars / 4))}
            </div>
            <div>
              <span>Limit for this call</span>
              {fmtTokens(c.max_tokens)}
            </div>
            <div>
              <span>This attempt</span>
              {fmtDuration(now - (c.call_started_ms ?? c.started_ms))}
            </div>
            <div>
              <span>Whole job</span>
              {fmtDuration(now - c.started_ms)}
            </div>
            <div>
              <span>Last data</span>
              {c.last_data_ms ? `${fmtDuration(idleMs)} ago` : 'none yet'}
            </div>
          </div>
          <p className="muted small">The answer usually ends well before the limit. Corydoras stops a call after 2 minutes without data.</p>
        </>
      )}
      {!inCall && (
        <div className="metrics">
          <div>
            <span>Whole job</span>
            {fmtDuration(now - c.started_ms)}
          </div>
        </div>
      )}
      {quiet && (
        <div className="review">
          No data for {fmtDuration(idleMs)}. A busy provider can be slow. At 2 minutes without data, Corydoras stops the call and
          tries again later.
        </div>
      )}
    </section>
  );
}

function jobTitle(j: JobView): string {
  if (j.kind === 'synthesize') return 'Write the language document';
  if (j.kind === 'analyze_source') return `Read “${j.item_name ?? 'source'}”`;
  return `Analyze “${j.item_name ?? 'image'}”`;
}

function whenText(j: JobView, all: JobView[], now: number, paused: boolean): string {
  if (j.running) return 'Running now';
  if (paused) return 'Paused';
  if (j.kind === 'synthesize' && all.some((o) => (o.kind === 'analyze_item' || o.kind === 'analyze_source') && o.language_id === j.language_id)) {
    return 'After the analyses';
  }
  if (j.run_after_ms - now < 1000) return 'Next';
  return `In ${fmtDuration(j.run_after_ms - now)}`;
}

function LogRow({ e, langName }: { e: LogEntry; langName: string | null }) {
  return (
    <li className={`log-row log-${e.level}`}>
      <span className="log-time">{clock(e.at_ms)}</span>
      <span className="dot" />
      <span className="log-text">
        {langName && <span className="tag-pill">{langName}</span>} {e.text}
      </span>
    </li>
  );
}

export default function ActivityView({ languages, onChanged, onOpenLanguage }: Props) {
  const toast = useToast();
  const [report, setReport] = useState<ActivityReport | null>(null);
  const [offset, setOffset] = useState(0);
  const [, setTick] = useState(0);
  const [confirmAll, setConfirmAll] = useState(false);
  const [onlyProblems, setOnlyProblems] = useState(false);

  const load = useCallback(() => {
    api
      .activity()
      .then((r) => {
        setReport(r);
        setOffset(Date.now() - r.now_ms);
      })
      .catch((e) => toast.error(errText(e)));
  }, [toast]);

  useEffect(() => {
    load();
    const t = setInterval(load, 1000);
    const tick = setInterval(() => setTick((x) => x + 1), 500);
    return () => {
      clearInterval(t);
      clearInterval(tick);
    };
  }, [load]);

  const now = Date.now() - offset;
  const nameOf = (id: string | null) => (id ? languages.find((l) => l.id === id)?.name ?? id : null);

  const cancel = async (jobId: number) => {
    try {
      await api.cancelJob(jobId);
      load();
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const cancelAll = async () => {
    setConfirmAll(false);
    try {
      const n = await api.cancelAll(null);
      toast.info(`Cancelled ${n} job${n === 1 ? '' : 's'}.`);
      load();
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const togglePause = async () => {
    if (!report) return;
    try {
      await api.setPaused(!report.paused);
      load();
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const waiting = report?.jobs.filter((j) => !j.running) ?? [];
  const log = (report?.log ?? []).filter((e) => !onlyProblems || e.level === 'warn' || e.level === 'error');

  return (
    <div className="settings">
      <header className="lang-head" data-tauri-drag-region>
        <div className="lang-title">
          <div className="eyebrow">Queue</div>
          <h1>Activity</h1>
          <p className="lang-desc muted">What Corydoras does now, what waits, and what happened.</p>
        </div>
        <div className="lang-actions">
          {report && (
            <button className={`btn small ${report.paused ? 'primary' : ''}`} onClick={togglePause}>
              {report.paused ? <Play size={13} /> : <Pause size={13} />}
              {report.paused ? 'Resume queue' : 'Pause queue'}
            </button>
          )}
          {confirmAll ? (
            <>
              <button className="btn small danger" onClick={cancelAll}>
                Cancel {report?.jobs.length ?? 0} job{report?.jobs.length === 1 ? '' : 's'}
              </button>
              <button className="btn small" onClick={() => setConfirmAll(false)}>
                Keep
              </button>
            </>
          ) : (
            <button className="btn small ghost-danger" onClick={() => setConfirmAll(true)} disabled={!report || report.jobs.length === 0}>
              <X size={13} /> Cancel all…
            </button>
          )}
        </div>
      </header>

      <div className="settings-body">
        {report?.paused && (
          <div className="notice">
            The queue is paused. A running job finishes, but no new job starts. Your choice stays after a restart.
          </div>
        )}

        {report?.current ? (
          <NowCard c={report.current} now={now} onCancel={() => cancel(report.current!.job_id)} />
        ) : (
          <section className="panel now-card idle">
            <h2>Nothing runs now.</h2>
            <p className="muted small">
              {waiting.length === 0
                ? 'The queue is empty. New images, sources and annotation changes add jobs automatically.'
                : report?.manual && !report.running
                  ? `${waiting.length} job${waiting.length === 1 ? '' : 's'} wait. Manual mode: click Run queue at the top right to start them.`
                  : report?.paused
                  ? `${waiting.length} job${waiting.length === 1 ? '' : 's'} wait until you resume the queue.`
                  : `${waiting.length} job${waiting.length === 1 ? '' : 's'} wait. The next one starts ${
                      waiting[0].run_after_ms <= now ? 'in a moment' : `in ${fmtDuration(waiting[0].run_after_ms - now)}`
                    }.`}
            </p>
          </section>
        )}

        <section className="panel">
          <h3>Up next</h3>
          {waiting.length === 0 ? (
            <p className="muted small">No job waits.</p>
          ) : (
            <table className="vtable qtable">
              <thead>
                <tr>
                  <th>Job</th>
                  <th>Language</th>
                  <th>When</th>
                  <th>Earlier tries</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {waiting.map((j) => (
                  <tr key={j.id}>
                    <td>{jobTitle(j)}</td>
                    <td>
                      <button className="link-btn" onClick={() => onOpenLanguage(j.language_id)}>
                        {j.language_name}
                      </button>
                    </td>
                    <td className="small">{whenText(j, report?.jobs ?? [], now, report?.paused ?? false)}</td>
                    <td className="small">
                      {j.attempts === 0 ? (
                        <span className="muted">—</span>
                      ) : (
                        <span title={j.last_error ?? ''}>
                          {j.attempts} failed{j.last_error ? `: ${j.last_error.slice(0, 90)}${j.last_error.length > 90 ? '…' : ''}` : ''}
                        </span>
                      )}
                    </td>
                    <td>
                      <button className="link-btn danger-link" onClick={() => cancel(j.id)}>
                        Cancel
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>

        <section className="panel">
          <div className="row" style={{ marginTop: 0 }}>
            <h3 className="grow" style={{ marginBottom: 0 }}>
              Recent activity
            </h3>
            <label className="check small">
              <input type="checkbox" checked={onlyProblems} onChange={(e) => setOnlyProblems(e.target.checked)} />
              Only problems
            </label>
          </div>
          {log.length === 0 ? (
            <p className="muted small">Nothing yet in this session. The log starts when the app opens.</p>
          ) : (
            <ul className="log">
              {log.map((e, i) => (
                <LogRow key={`${e.at_ms}-${i}`} e={e} langName={nameOf(e.language_id)} />
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}
