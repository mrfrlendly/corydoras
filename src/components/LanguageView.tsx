import { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent, useEffect, useRef, useState } from 'react';
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
  /** Manual queue mode: new jobs wait for Run queue. */
  manual?: boolean;
}

/** A pass that asks before it starts: every reference, or the analyses of another model. */
type ConfirmPass = 'all' | 'other-model';

/** The banner of another model is hidden while this stays the same. */
const otherSig = (o: OtherModelView) => `${o.model}|${o.other.images + o.other.sources}`;

function loadHiddenSig(languageId: string): string | null {
  try {
    return localStorage.getItem(`cory.otherModelHidden.${languageId}`);
  } catch {
    return null;
  }
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
  return <span className="chip chip-ok">Up to date</span>;
}

export default function LanguageView({
  language,
  languages,
  projects,
  status,
  worker,
  current,
  paused,
  tick,
  onChanged,
  onSettings,
  onActivity,
  manual = false,
}: Props) {
  const toast = useToast();
  const [tab, setTab] = useState<Tab>('board');
  const [docVersion, setDocVersion] = useState<number | null>(null);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(language.name);
  const [description, setDescription] = useState(language.description);
  const [base, setBase] = useState(language.base_id ?? '');
  const [baseVersion, setBaseVersion] = useState<number | null>(language.base_version);
  const [baseVersions, setBaseVersions] = useState<VersionInfo[]>([]);
  const [confirm, setConfirm] = useState<ConfirmPass | null>(null);
  const [est, setEst] = useState<Estimate | null>(null);
  const [estFailed, setEstFailed] = useState(false);
  const [queueEstimate, setQueueEstimate] = useState<Estimate | null>(null);
  const [other, setOther] = useState<OtherModelView | null>(null);
  const [hiddenSig, setHiddenSig] = useState<string | null>(() => loadHiddenSig(language.id));
  // An Insight link asks the Board to show one reference.
  const [focusItem, setFocusItem] = useState<string | null>(null);
  const confirmFrom = useRef<HTMLElement | null>(null);
  const askSeq = useRef(0);
  const editBtnRef = useRef<HTMLButtonElement>(null);
  const refocusEdit = useRef(false);
  const tabRefs = useRef<Partial<Record<Tab, HTMLButtonElement | null>>>({});

  const baseLanguage = languages.find((l) => l.id === language.base_id) ?? null;
  const waiting = status ? status.pending_items > 0 || status.synth_due_in !== null : false;
  const refCount = language.item_count + language.source_count;
  const otherCount = other ? other.other.images + other.other.sources : 0;
  const showOther = !!other && otherSig(other) !== hiddenSig;
  // Manual mode: the worker says so while it waits for Run queue.
  const manualWait = manual || (worker?.state === 'paused' && worker.message.startsWith('Manual'));

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
      .then((o) => alive && setOther(o.other.images + o.other.sources > 0 ? o : null))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [language.id, tick]);

  // A confirm for analyses that are gone (all were redone) closes.
  useEffect(() => {
    if (confirm === 'other-model' && !other) setConfirm(null);
  }, [confirm, other]);

  // Escape cancels the confirm bar, before any other Escape (the inspector, a form).
  useEffect(() => {
    if (!confirm) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      e.preventDefault();
      closeConfirm();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
    // closeConfirm only uses refs and state setters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [confirm]);

  // Back to "Edit details" after the form closes from the keyboard.
  useEffect(() => {
    if (!editing && refocusEdit.current) {
      refocusEdit.current = false;
      editBtnRef.current?.focus();
    }
  }, [editing]);

  const ask = (p: ConfirmPass, e: ReactMouseEvent<HTMLElement>) => {
    confirmFrom.current = e.currentTarget;
    const seq = ++askSeq.current;
    setConfirm(p);
    setEst(null);
    setEstFailed(false);
    api
      .estimate(language.id, p)
      .then((x) => seq === askSeq.current && setEst(x))
      .catch(() => seq === askSeq.current && setEstFailed(true));
  };

  const closeConfirm = () => {
    askSeq.current++;
    setConfirm(null);
    const from = confirmFrom.current;
    confirmFrom.current = null;
    if (from && from.isConnected) from.focus();
  };

  const hideOther = () => {
    if (!other) return;
    const sig = otherSig(other);
    setHiddenSig(sig);
    try {
      localStorage.setItem(`cory.otherModelHidden.${language.id}`, sig);
    } catch {
      // Storage is a convenience only.
    }
  };

  const goTab = (t: Tab) => {
    setTab(t);
    setFocusItem(null);
  };

  const onTabKey = (e: ReactKeyboardEvent<HTMLElement>) => {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const i = TABS.findIndex((x) => x.id === tab);
    const n =
      e.key === 'ArrowRight'
        ? (i + 1) % TABS.length
        : e.key === 'ArrowLeft'
          ? (i - 1 + TABS.length) % TABS.length
          : e.key === 'Home'
            ? 0
            : e.key === 'End'
              ? TABS.length - 1
              : -1;
    if (n < 0) return;
    e.preventDefault();
    goTab(TABS[n].id);
    tabRefs.current[TABS[n].id]?.focus();
  };

  const cancelEdit = () => {
    setEditing(false);
    setName(language.name);
    setDescription(language.description);
    setBase(language.base_id ?? '');
    setBaseVersion(language.base_version);
  };

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

  const analyzeAll = async () => {
    closeConfirm();
    try {
      const n = await api.reanalyzeAll(language.id);
      toast.info(`${n} item${n === 1 ? '' : 's'} queued for analysis.${language.base_id ? '' : ' A new version follows.'}`);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const analyzeOther = async () => {
    closeConfirm();
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
      toast.info(
        status && status.pending_items > 0
          ? 'The new version is written after the item analysis finishes.'
          : language.latest_version
            ? `If nothing changed since v${language.latest_version}, no call is made.`
            : 'The first version is written next.',
      );
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
            <form
              className="meta-edit"
              onSubmit={(e) => {
                e.preventDefault();
                saveMeta();
              }}
              onKeyDown={(e) => {
                if (e.defaultPrevented) return;
                if (e.key === 'Escape') {
                  e.preventDefault();
                  refocusEdit.current = true;
                  cancelEdit();
                } else if (e.key === 'Enter' && ((e.target as HTMLElement).tagName !== 'TEXTAREA' || e.metaKey || e.ctrlKey)) {
                  refocusEdit.current = true;
                }
              }}
            >
              <input
                className="title-input"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Name"
                aria-label="Name"
                autoFocus
              />
              <textarea
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault();
                    e.currentTarget.form?.requestSubmit();
                  }
                }}
                placeholder="Describe the intent of this language in one or two sentences. The analysis reads this text."
                aria-label="Description"
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
                <button className="btn primary small" type="submit">
                  Save
                </button>
                <button className="btn small" type="button" onClick={cancelEdit}>
                  Cancel
                </button>
              </div>
            </form>
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
            <button ref={editBtnRef} className="btn small" onClick={() => setEditing(true)} title="Edit the name, the description and the base language">
              <Pencil size={13} /> Edit details
            </button>
          )}
          <button
            className="btn small"
            onClick={(e) => ask('all', e)}
            disabled={refCount === 0}
            aria-expanded={confirm === 'all'}
            title="Analyze every image and source again. The next step shows what it uses."
          >
            <RefreshCw size={13} /> Analyze all again
          </button>
          <button
            className="btn small"
            onClick={synthNow}
            disabled={language.item_count === 0 || !!language.base_id}
            title={
              language.base_id
                ? `This language extends ${baseLanguage?.name ?? 'a base'}: its document comes from the base and your changes.`
                : 'Write the next version now instead of waiting. When nothing changed, no call is made.'
            }
          >
            <Sparkles size={13} /> Synthesize now
          </button>
        </div>
      </header>

      {(confirm || showOther) && (
        <div className="lang-notes">
          {confirm && (
            <div key={confirm} className="notice confirm-bar" role="group" aria-label="Confirm">
              <span className="confirm-text">
                {confirm === 'all'
                  ? `Analyze ${refCount} reference${refCount === 1 ? '' : 's'} again${language.base_id ? '' : ', then write a new version'}.`
                  : `Analyze ${otherCount === 1 ? 'the older analysis' : `the ${otherCount} older analyses`} again with ${other?.model ?? 'the current model'}${
                      language.base_id ? '' : ', then write a new version'
                    }.`}{' '}
                {est ? `It uses ${fmtEstimate(est)}.` : estFailed ? 'The cost is not known.' : 'Counting the cost…'}
              </span>
              <button
                className="btn small primary"
                onClick={confirm === 'all' ? analyzeAll : analyzeOther}
                disabled={!est && !estFailed}
                title="Uses one model call for each reference"
              >
                Analyze {confirm === 'all' ? refCount : otherCount} again
              </button>
              <button className="btn small" onClick={closeConfirm} autoFocus>
                Cancel
              </button>
            </div>
          )}
          {other && showOther && (
            <div className="notice warn model-notice">
              <span className="notice-text">
                {otherCount} analys{otherCount === 1 ? 'is is' : 'es are'} from {other.other.models.length > 0 ? other.other.models.join(', ') : 'another model'}. New
                images go to <b>{other.model}</b>, so the notes are not all from one model.
              </span>
              <button className="btn small" onClick={(e) => ask('other-model', e)} aria-expanded={confirm === 'other-model'}>
                Analyze {otherCount === 1 ? 'it' : `the ${otherCount}`} again…
              </button>
              <button className="icon-btn" title="Hide until the count or the model changes" aria-label="Hide" onClick={hideOther}>
                <X size={13} />
              </button>
            </div>
          )}
        </div>
      )}

      <nav className="tabs" role="tablist" aria-label="Language" onKeyDown={onTabKey}>
        {TABS.map(({ id: t, label, icon: Icon }) => (
          <button
            key={t}
            ref={(el) => {
              tabRefs.current[t] = el;
            }}
            id={`tab-${t}`}
            role="tab"
            aria-selected={tab === t}
            aria-controls="lang-panel"
            tabIndex={tab === t ? 0 : -1}
            className={`tab ${tab === t ? 'active' : ''}`}
            onClick={() => goTab(t)}
          >
            <Icon size={14} strokeWidth={1.8} />
            {label}
            {t === 'board' && language.item_count > 0 ? <span className="tab-badge">{language.item_count}</span> : null}
            {t === 'sources' && language.source_count > 0 ? <span className="tab-badge">{language.source_count}</span> : null}
            {t === 'outputs' && status && status.pending_outputs > 0 ? (
              <>
                <span className="tab-dot" aria-hidden="true" title={`${status.pending_outputs} output${status.pending_outputs === 1 ? '' : 's'} waiting for a verdict`}>
                  {status.pending_outputs}
                </span>
                <span className="sr-only">, {status.pending_outputs} waiting for a verdict</span>
              </>
            ) : null}
            {t === 'document' && language.latest_version ? <span className="tab-badge">v{docVersion ?? language.latest_version}</span> : null}
            {t === 'document' && status && status.open_proposals > 0 ? (
              <>
                <span className="tab-dot" aria-hidden="true" title={`${status.open_proposals} proposed amendment${status.open_proposals === 1 ? '' : 's'}`}>
                  {status.open_proposals}
                </span>
                <span className="sr-only">
                  , {status.open_proposals} proposed amendment{status.open_proposals === 1 ? '' : 's'}
                </span>
              </>
            ) : null}
          </button>
        ))}
      </nav>

      <div className="lang-body" role="tabpanel" id="lang-panel" aria-labelledby={`tab-${tab}`}>
        {tab === 'board' && (
          <Board
            language={language}
            tick={tick}
            onChanged={onChanged}
            onSources={() => goTab('sources')}
            waits={manualWait ? 'manual' : paused ? 'paused' : null}
            focus={focusItem}
          />
        )}
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
              goTab('document');
            }}
            onChanged={onChanged}
          />
        )}
        {tab === 'insight' && (
          <InsightView
            language={language}
            tick={tick}
            onBoard={(id) => {
              setFocusItem(id);
              setTab('board');
            }}
            onDocument={() => {
              setDocVersion(null);
              goTab('document');
            }}
            onActivity={onActivity}
          />
        )}
      </div>
    </div>
  );
}
