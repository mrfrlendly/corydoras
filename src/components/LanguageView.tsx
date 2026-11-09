import { useEffect, useState } from 'react';
import { api, CurrentJob, errText, Language, LanguageStatus, shortStatus, StatusReport } from '../api';
import { BookOpen, ClipboardCheck, FileText, History, LayoutGrid, Pencil, RefreshCw, Sparkles, X } from 'lucide-react';
import Board from './Board';
import SourcesView from './SourcesView';
import DocumentView from './DocumentView';
import VersionsView from './VersionsView';
import OutputsView from './OutputsView';
import { useToast } from './Toast';

type Tab = 'board' | 'sources' | 'outputs' | 'document' | 'versions';

const TABS: { id: Tab; label: string; icon: typeof LayoutGrid }[] = [
  { id: 'board', label: 'Board', icon: LayoutGrid },
  { id: 'sources', label: 'Sources', icon: BookOpen },
  { id: 'outputs', label: 'Outputs', icon: ClipboardCheck },
  { id: 'document', label: 'Document', icon: FileText },
  { id: 'versions', label: 'Versions', icon: History },
];

interface Props {
  language: Language;
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

export default function LanguageView({ language, status, worker, current, paused, tick, onChanged, onSettings, onActivity }: Props) {
  const toast = useToast();
  const [tab, setTab] = useState<Tab>('board');
  const [docVersion, setDocVersion] = useState<number | null>(null);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(language.name);
  const [description, setDescription] = useState(language.description);
  const [confirmAll, setConfirmAll] = useState(false);

  useEffect(() => {
    setName(language.name);
    setDescription(language.description);
  }, [language.name, language.description]);

  const saveMeta = async () => {
    try {
      await api.updateLanguage(language.id, name, description);
      setEditing(false);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const analyzeAll = async () => {
    setConfirmAll(false);
    try {
      const n = await api.reanalyzeAll(language.id);
      toast.info(`${n} item${n === 1 ? '' : 's'} queued for analysis. A new version follows.`);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const synthNow = async () => {
    try {
      await api.synthesizeNow(language.id);
      toast.info('The synthesis pass starts after the item analysis finishes.');
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
          {!editing && (
            <button className="btn small" onClick={() => setEditing(true)} title="Edit the name and the description">
              <Pencil size={13} /> Edit
            </button>
          )}
          {confirmAll ? (
            <>
              <button className="btn small primary" onClick={analyzeAll} title="Uses one model call for each item">
                Analyze {language.item_count + language.source_count} again
              </button>
              <button className="btn small" onClick={() => setConfirmAll(false)}>
                Cancel
              </button>
            </>
          ) : (
            <button
              className="btn small"
              onClick={() => setConfirmAll(true)}
              disabled={language.item_count + language.source_count === 0}
              title="Analyze every image and source again"
            >
              <RefreshCw size={13} /> Analyze all again
            </button>
          )}
          <button className="btn small" onClick={synthNow} disabled={language.item_count === 0} title="Write a new version of the language document">
            <Sparkles size={13} /> Synthesize now
          </button>
        </div>
      </header>

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
            tick={tick}
            onOpen={(v) => {
              setDocVersion(v);
              setTab('document');
            }}
          />
        )}
      </div>
    </div>
  );
}
