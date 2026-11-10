import { useEffect, useState } from 'react';
import { api, CurrentJob, errText, Estimate, fmtEstimate, Language, LanguageStatus, OtherModelView, Project, shortStatus, StatusReport, VersionInfo } from '../api';
import { BookOpen, ClipboardCheck, FileText, History, LayoutGrid, Pencil, RefreshCw, ScanSearch, Sparkles, X } from 'lucide-react';
import Board from './Board';
import SourcesView from './SourcesView';
import DocumentView from './DocumentView';
import VersionsView from './VersionsView';
import OutputsView from './OutputsView';
import InsightView from './InsightView';
import { useToast } from './Toast';

type Tab = 'board' | 'sources' | 'outputs' | 'document' | 'versions' | 'insight';

const TABS: { id: Tab; label: string; icon: typeof LayoutGrid }[] = [
  { id: 'board', label: 'Board', icon: LayoutGrid },
  { id: 'sources', label: 'Sources', icon: BookOpen },
  { id: 'outputs', label: 'Outputs', icon: ClipboardCheck },
  { id: 'document', label: 'Document', icon: FileText },
  { id: 'versions', label: 'Versions', icon: History },
  { id: 'insight', label: 'Insight', icon: ScanSearch },
];

interface Props {
  language: Language;
  /** All languages: one of them can be the base of this one. */
  languages: Language[];
  projects: Project[];
  status: LanguageStatus | null;
  worker: StatusReport['worker'] | null;
  current: CurrentJob | null;
  paused: boolean;
  tick: number;
  onChanged: () => void;
  onSettings: () => void;
  onActivity: () => void;
}

function StatusChip({
  status,
  worker,
  current,
  paused,
  onSettings,
  onRetry,
  onActivity,
  onCancel,
}: {
  status: LanguageStatus | null;
  worker: StatusReport['worker'] | null;
  current: CurrentJob | null;
  paused: boolean;
  onSettings: () => void;
  onRetry: () => void;
  onActivity: () => void;
  onCancel: () => void;
}) {
  if (current) {
    return (
      <span className="chip chip-busy chip-live">
        <button className="chip-main" onClick={onActivity} title="Open Activity">
          {shortStatus(current, Date.now())}
        </button>
        <button className="chip-x" onClick={onCancel} title="Cancel this job" disabled={current.cancel_requested}>
          <X size={12} />
        </button>
      </span>
    );
  }
  const waiting = status ? status.pending_items > 0 || status.synth_due_in !== null : false;
  if (paused && waiting) {
    return (
      <button className="chip chip-warn" onClick={onActivity}>
        Queue paused
      </button>
    );
  }
  if (worker?.state === 'paused' && waiting && worker.message.startsWith('Manual')) {
    return (
      <button className="chip" onClick={onActivity} title="Manual mode: click Run queue at the top right">
        Waiting for Run queue
      </button>
    );
  }
  if (worker?.state === 'paused' && waiting) {
    return (
      <button className="chip chip-warn" onClick={onSettings} title={worker.message}>
        Paused · {worker.message}
      </button>
    );
  }
  if (!status) return null;
  if (status.pending_items > 0) {
    return (
      <button className="chip" onClick={onActivity}>
        {status.pending_items} waiting for analysis
      </button>
    );
  }
  if (status.synth_due_in !== null) {
    return (
      <button className="chip" onClick={onActivity}>
        {status.synth_due_in > 0 ? `New version in ${status.synth_due_in} s` : 'New version waits in the queue'}
      </button>
    );
  }
  if (status.error_items > 0) {
    return (
      <button className="chip chip-error" onClick={onRetry} title="Queue the failed items again">
        {status.error_items} failed · Retry
      </button>
    );
  }
  return <span className="chip">Up to date</span>;
}

export default function LanguageView({ language, languages, projects, status, worker, current, paused, tick, onChanged, onSettings, onActivity }: Props) {
  const toast = useToast();
  const [tab, setTab] = useState<Tab>('board');
  const [docVersion, setDocVersion] = useState<number | null>(null);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(language.name);
  const [description, setDescription] = useState(language.description);
  const [base, setBase] = useState(language.base_id ?? '');
  const [baseVersion, setBaseVersion] = useState<number | null>(language.base_version);
  const [baseVersions, setBaseVersions] = useState<VersionInfo[]>([]);
  const [confirmAll, setConfirmAll] = useState(false);
  const [allEstimate, setAllEstimate] = useState<Estimate | null>(null);
  const [queueEstimate, setQueueEstimate] = useState<Estimate | null>(null);
  const [docEstimate, setDocEstimate] = useState<Estimate | null>(null);
  const [other, setOther] = useState<OtherModelView | null>(null);
  const [otherEstimate, setOtherEstimate] = useState<Estimate | null>(null);

  const baseLanguage = languages.find((l) => l.id === language.base_id) ?? null;
  const waiting = status ? status.pending_items > 0 || status.synth_due_in !== null : false;

  useEffect(() => {
    setName(language.name);
    setDescription(language.description);
    setBase(language.base_id ?? '');
    setBaseVersion(language.base_version);
  }, [language.name, language.description, language.base_id, language.base_version]);

  // The versions of the base that the edit form offers.
  useEffect(() => {
    if (!editing || !base) {
      setBaseVersions([]);
      return;
    }
    let alive = true;
    api
      .listVersions(base)
      .then((v) => alive && setBaseVersions(v.filter((x) => x.schema >= 2)))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [editing, base]);

  // What the waiting passes will use, before they run.
  useEffect(() => {
    if (!waiting || current) {
      setQueueEstimate(null);
      return;
    }
    let alive = true;
    api
      .estimate(language.id, 'queue')
      .then((e) => alive && setQueueEstimate(e.total_tokens > 0 ? e : null))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [language.id, waiting, current, tick]);

  // Analyses that another model wrote (the provider changed since).
  useEffect(() => {
    let alive = true;
    api
      .otherModel(language.id)
      .then((o) => {
        if (!alive) return;
        const stale = o.other.images + o.other.sources > 0;
        setOther(stale ? o : null);
        if (stale) {
          api
            .estimate(language.id, 'other-model')
            .then((e) => alive && setOtherEstimate(e))
            .catch(() => {});
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [language.id, tick]);

  const saveMeta = async () => {
    try {
      await api.updateLanguage(language.id, name, description);
      if (base !== (language.base_id ?? '') || (base && baseVersion !== language.base_version)) {
        await api.setBase(language.id, base || null, base ? baseVersion : null);
        toast.info(
          base
            ? `${name} now extends ${languages.find((l) => l.id === base)?.name ?? base}. Its document is the base with your changes.`
            : `${name} extends no language now. Its versions stay.`,
        );
        setDocVersion(null);
      }
      setEditing(false);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const askAnalyzeAll = () => {
    setConfirmAll(true);
    setAllEstimate(null);
    api
      .estimate(language.id, 'all')
      .then(setAllEstimate)
      .catch(() => {});
  };

  const analyzeAll = async () => {
    setConfirmAll(false);
    try {
      const n = await api.reanalyzeAll(language.id);
      toast.info(`${n} item${n === 1 ? '' : 's'} queued for analysis.${language.base_id ? '' : ' A new version follows.'}`);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const analyzeOther = async () => {
    try {
      const n = await api.reanalyzeOtherModel(language.id);
      toast.info(`${n} item${n === 1 ? '' : 's'} queued for analysis with ${other?.model ?? 'the current model'}.`);
      setOther(null);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const synthNow = async () => {
    try {
      await api.synthesizeNow(language.id);
      toast.info(`The synthesis pass starts after the item analysis finishes.${docEstimate ? ` It uses ${fmtEstimate(docEstimate)}.` : ''}`);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  return (
    <div className="lang">
      <header className="lang-head" data-tauri-drag-region>
        <div className="lang-title" data-tauri-drag-region>
          {editing ? (
            <div className="meta-edit">
              <input className="title-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" autoFocus />
              <textarea
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Describe the intent of this language in one or two sentences. The analysis reads this text."
                rows={2}
              />
              <div className="row base-row">
                <label className="small muted" htmlFor="base-select">
                  Extends
                </label>
                <select id="base-select" className="select small" value={base} onChange={(e) => (setBase(e.target.value), setBaseVersion(null))}>
                  <option value="">No base: this language has its own document</option>
                  {languages
                    .filter((l) => l.id !== language.id && l.latest_version !== null)
                    .map((l) => (
                      <option key={l.id} value={l.id}>
                        {l.name}
                      </option>
                    ))}
                </select>
                {base && (
                  <select
                    className="select small"
                    value={baseVersion ?? ''}
                    onChange={(e) => setBaseVersion(e.target.value ? Number(e.target.value) : null)}
                    aria-label="Version of the base"
                  >
                    <option value="">Follow the latest version</option>
                    {baseVersions.map((v) => (
                      <option key={v.version} value={v.version}>
                        Stay on v{v.version}
                      </option>
                    ))}
                  </select>
                )}
              </div>
              {base && (
                <p className="muted small">
                  The document of this language becomes the document of the base, with the changes that you make in the Document tab. It gets no
                  synthesis of its own; its images stay as references for agents.
                  {base !== (language.base_id ?? '') && language.override_count > 0 ? ' A new base starts without the changes that you made to the old one.' : ''}
                </p>
              )}
              <div className="row">
                <button className="btn primary small" onClick={saveMeta}>
                  Save
                </button>
                <button
                  className="btn small"
                  onClick={() => {
                    setEditing(false);
                    setName(language.name);
                    setDescription(language.description);
                  }}
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : (
            <>
              <div className="eyebrow">
                {language.id} · {language.item_count} image{language.item_count === 1 ? '' : 's'}
                {language.source_count > 0 ? ` · ${language.source_count} source${language.source_count === 1 ? '' : 's'}` : ''}
                {language.latest_version ? ` · v${language.latest_version}` : ''}
                {language.base_id ? ` · extends ${baseLanguage?.name ?? language.base_id}` : ''}
              </div>
              <h1 onDoubleClick={() => setEditing(true)} title="Double-click to edit">
                {language.name}
              </h1>
              <p className={`lang-desc ${language.description ? '' : 'muted'}`} onDoubleClick={() => setEditing(true)}>
                {language.description || 'No description. Double-click to add the intent of this language.'}
              </p>
            </>
          )}
        </div>
        <div className="lang-actions">
          <StatusChip
            status={status}
            worker={worker}
            current={current}
            paused={paused}
            onSettings={onSettings}
            onActivity={onActivity}
            onCancel={() =>
              current &&
              api
                .cancelJob(current.job_id)
                .then(onChanged)
                .catch((e) => toast.error(errText(e)))
            }
            onRetry={() =>
              api
                .retryFailed(language.id)
                .then((n) => {
                  toast.info(`${n} item${n === 1 ? '' : 's'} queued again.`);
                  onChanged();
                })
                .catch((e) => toast.error(errText(e)))
            }
          />
          {queueEstimate && (
            <span
              className="chip"
              title={`The passes that wait will use ${fmtEstimate(queueEstimate)}: ${queueEstimate.images.calls} image and ${queueEstimate.sources.calls} source analyses${queueEstimate.document.calls ? ', then the document' : ''}. ${queueEstimate.from_history ? 'From earlier calls in this library.' : 'From typical values; the first runs make it exact.'}`}
            >
              ≈ {fmtEstimate(queueEstimate).replace('about ', '')}
            </span>
          )}
          {!editing && (
            <button className="btn small" onClick={() => setEditing(true)} title="Edit the name, the description and the base language">
              <Pencil size={13} /> Edit
            </button>
          )}
          {confirmAll ? (
            <>
              <button className="btn small primary" onClick={analyzeAll} title="Uses one model call for each item">
                Analyze {language.item_count + language.source_count} again{allEstimate ? ` · ${fmtEstimate(allEstimate)}` : ''}
              </button>
              <button className="btn small" onClick={() => setConfirmAll(false)}>
                Cancel
              </button>
            </>
          ) : (
            <button
              className="btn small"
              onClick={askAnalyzeAll}
              disabled={language.item_count + language.source_count === 0}
              title="Analyze every image and source again. The next step shows what it uses."
            >
              <RefreshCw size={13} /> Analyze all again
            </button>
          )}
          <button
            className="btn small"
            onClick={synthNow}
            onMouseEnter={() =>
              !language.base_id &&
              language.item_count > 0 &&
              api
                .estimate(language.id, 'document')
                .then(setDocEstimate)
                .catch(() => {})
            }
            disabled={language.item_count === 0 || !!language.base_id}
            title={
              language.base_id
                ? `This language extends ${baseLanguage?.name ?? 'a base'}: its document comes from the base and your changes.`
                : `Write a new version of the language document${docEstimate ? `: ${fmtEstimate(docEstimate)}` : ''}`
            }
          >
            <Sparkles size={13} /> Synthesize now
          </button>
        </div>
      </header>

      {other && (
        <div className="notice model-notice">
          {other.other.images + other.other.sources} analys{other.other.images + other.other.sources === 1 ? 'is was' : 'es were'} written by{' '}
          {other.other.models.length > 0 ? other.other.models.join(', ') : 'another model'}. Images now go to <b>{other.model}</b>, so the notes of the
          references are not all from one model.{' '}
          <button className="link-btn" onClick={analyzeOther}>
            Analyze {other.other.images + other.other.sources === 1 ? 'it' : 'them'} again
          </button>
          {otherEstimate && <span className="muted">{fmtEstimate(otherEstimate)}</span>}
          <button className="icon-btn" title="Hide until the next change" aria-label="Hide" onClick={() => setOther(null)}>
            <X size={13} />
          </button>
        </div>
      )}

      <nav className="tabs">
        {TABS.map(({ id: t, label, icon: Icon }) => (
          <button key={t} className={`tab ${tab === t ? 'active' : ''}`} onClick={() => setTab(t)}>
            <Icon size={14} strokeWidth={1.8} />
            {label}
            {t === 'board' && language.item_count > 0 ? <span className="tab-badge">{language.item_count}</span> : null}
            {t === 'sources' && language.source_count > 0 ? <span className="tab-badge">{language.source_count}</span> : null}
            {t === 'outputs' && status && status.pending_outputs > 0 ? (
              <span className="tab-dot" title={`${status.pending_outputs} output${status.pending_outputs === 1 ? '' : 's'} waiting for a verdict`}>
                {status.pending_outputs}
              </span>
            ) : null}
            {t === 'document' && language.latest_version ? <span className="tab-badge">v{docVersion ?? language.latest_version}</span> : null}
            {t === 'document' && status && status.open_proposals > 0 ? (
              <span className="tab-dot" title={`${status.open_proposals} proposed amendment${status.open_proposals === 1 ? '' : 's'}`}>
                {status.open_proposals}
              </span>
            ) : null}
          </button>
        ))}
      </nav>

      <div className="lang-body">
        {tab === 'board' && <Board language={language} tick={tick} onChanged={onChanged} onSources={() => setTab('sources')} />}
        {tab === 'sources' && <SourcesView language={language} tick={tick} onChanged={onChanged} />}
        {tab === 'outputs' && <OutputsView language={language} tick={tick} pending={status?.pending_outputs ?? 0} onChanged={onChanged} />}
        {tab === 'document' && (
          <DocumentView
            language={language}
            version={docVersion}
            tick={tick}
            openProposals={status?.open_proposals ?? 0}
            onVersion={setDocVersion}
            onChanged={onChanged}
          />
        )}
        {tab === 'versions' && (
          <VersionsView
            language={language}
            projects={projects}
            tick={tick}
            onOpen={(v) => {
              setDocVersion(v);
              setTab('document');
            }}
            onChanged={onChanged}
          />
        )}
        {tab === 'insight' && <InsightView language={language} tick={tick} />}
      </div>
    </div>
  );
}
