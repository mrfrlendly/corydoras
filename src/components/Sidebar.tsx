import { FormEvent, useState } from 'react';
import { api, errText, fmtTokens, Language, Project, RuleSet, shortStatus, StatusReport } from '../api';
import type { View } from '../App';
import { Activity, Gauge, Layers, Pause, Plus, Settings, ShieldCheck, Swatch } from './icons';
import Logo from './Logo';
import { useToast } from './Toast';

interface Props {
  languages: Language[];
  projects: Project[];
  ruleSets: RuleSet[];
  status: StatusReport | null;
  view: View;
  onView: (v: View) => void;
  onChanged: () => void;
}

function NewForm({ placeholder, onSubmit, onCancel }: { placeholder: string; onSubmit: (name: string) => Promise<void>; onCancel: () => void }) {
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    setBusy(true);
    try {
      await onSubmit(name.trim());
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="side-new" onSubmit={submit}>
      <input
        autoFocus
        value={name}
        placeholder={placeholder}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => e.key === 'Escape' && onCancel()}
        disabled={busy}
      />
    </form>
  );
}

export default function Sidebar({ languages, projects, ruleSets, status, view, onView, onChanged }: Props) {
  const toast = useToast();
  const [newLang, setNewLang] = useState(false);
  const [newProject, setNewProject] = useState(false);
  const [newRuleSet, setNewRuleSet] = useState(false);

  const statusOf = (id: string) => status?.languages.find((s) => s.language_id === id);
  const used = status ? status.usage.input_tokens + status.usage.output_tokens : 0;

  return (
    <aside className="sidebar">
      <div className="side-top" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <Logo size={38} />
          <span className="brand-word">Corydoras</span>
        </div>
      </div>

      <div className="side-scroll">
        <div className="side-head">
          <span className="side-head-label">
            <Swatch size={13} strokeWidth={1.8} />
            Languages
          </span>
          <button className="icon-btn" title="New language" onClick={() => setNewLang(true)}>
            <Plus size={15} />
          </button>
        </div>
        {newLang && (
          <NewForm
            placeholder="Language name, then Return"
            onCancel={() => setNewLang(false)}
            onSubmit={async (name) => {
              try {
                const l = await api.createLanguage(name, '');
                setNewLang(false);
                onChanged();
                onView({ kind: 'language', id: l.id });
              } catch (e) {
                toast.error(errText(e));
              }
            }}
          />
        )}
        {languages.length === 0 && !newLang && <div className="side-empty">No languages yet.</div>}
        {languages.map((l) => {
          const st = statusOf(l.id);
          const active = view.kind === 'language' && view.id === l.id;
          return (
            <button key={l.id} className={`side-item ${active ? 'active' : ''}`} onClick={() => onView({ kind: 'language', id: l.id })}>
              <span className="side-name">{l.name}</span>
              <span className="side-meta">
                {st && st.pending_items > 0 && <span className="dot dot-busy" title={`${st.pending_items} in analysis`} />}
                {st && st.error_items > 0 && <span className="dot dot-error" title={`${st.error_items} failed`} />}
                {l.latest_version ? `v${l.latest_version}` : '—'}
              </span>
            </button>
          );
        })}

        <div className="side-head">
          <span className="side-head-label">
            <Layers size={13} strokeWidth={1.8} />
            Projects
          </span>
          <button className="icon-btn" title="New project" onClick={() => setNewProject(true)}>
            <Plus size={15} />
          </button>
        </div>
        {newProject && (
          <NewForm
            placeholder="Project name, then Return"
            onCancel={() => setNewProject(false)}
            onSubmit={async (name) => {
              try {
                const p = await api.createProject(name);
                setNewProject(false);
                onChanged();
                onView({ kind: 'project', id: p.id });
              } catch (e) {
                toast.error(errText(e));
              }
            }}
          />
        )}
        {projects.length === 0 && !newProject && <div className="side-empty">No projects yet.</div>}
        {projects.map((p) => {
          const active = view.kind === 'project' && view.id === p.id;
          const primary = p.pins.find((x) => x.role === 'primary');
          return (
            <button key={p.id} className={`side-item ${active ? 'active' : ''}`} onClick={() => onView({ kind: 'project', id: p.id })}>
              <span className="side-name">{p.name}</span>
              <span className="side-meta" title={primary ? `${primary.language_name} v${primary.version}` : ''}>
                {primary ? `v${primary.version}` : 'no pin'}
              </span>
            </button>
          );
        })}

        <div className="side-head">
          <span className="side-head-label">
            <ShieldCheck size={13} strokeWidth={1.8} />
            Rule sets
          </span>
          <button className="icon-btn" title="New rule set" onClick={() => setNewRuleSet(true)}>
            <Plus size={15} />
          </button>
        </div>
        {newRuleSet && (
          <NewForm
            placeholder="Rule set name, then Return"
            onCancel={() => setNewRuleSet(false)}
            onSubmit={async (name) => {
              try {
                const r = await api.createRuleSet({ name, description: '', always: false, rules: [] });
                setNewRuleSet(false);
                onChanged();
                onView({ kind: 'ruleset', id: r.id });
              } catch (e) {
                toast.error(errText(e));
              }
            }}
          />
        )}
        {ruleSets.length === 0 && !newRuleSet && <div className="side-empty">No rule sets yet.</div>}
        {ruleSets.map((r) => {
          const active = view.kind === 'ruleset' && view.id === r.id;
          return (
            <button key={r.id} className={`side-item ${active ? 'active' : ''}`} onClick={() => onView({ kind: 'ruleset', id: r.id })}>
              <span className="side-name">{r.name}</span>
              <span className="side-meta" title={r.always ? 'Applies to every project' : `${r.project_count} project(s)`}>
                {r.always ? <span className="always-pill">always</span> : `${r.rules.length}`}
              </span>
            </button>
          );
        })}
      </div>

      <div className="side-foot">
        <button
          className={`worker worker-btn ${status?.current ? 'worker-working' : status?.paused || status?.worker.state === 'paused' ? 'worker-paused' : 'worker-idle'} ${view.kind === 'activity' ? 'active' : ''}`}
          onClick={() => onView({ kind: 'activity' })}
          title="Open Activity"
        >
          <span className="worker-icon">
            {status?.paused && !status.current ? <Pause size={15} /> : <Activity size={15} />}
            <span className="dot" />
          </span>
          <span className="worker-text">
            {status?.current ? (
              <>
                <span className="worker-title">{status.current.title}</span>
                <span className="worker-sub">{shortStatus(status.current, Date.now())}</span>
              </>
            ) : status?.manual && !status.running ? (
              `Manual · ${status.queued} job${status.queued === 1 ? '' : 's'} waiting`
            ) : status?.paused ? (
              `Queue paused${status.queued > 0 ? ` · ${status.queued} waiting` : ''}`
            ) : status?.worker.state === 'paused' ? (
              status.worker.message
            ) : status && status.queued > 0 ? (
              `${status.queued} job${status.queued === 1 ? '' : 's'} waiting`
            ) : (
              'Queue is empty'
            )}
          </span>
        </button>
        {status && (
          <div className="usage" title={`Tokens in ${status.usage.month}`}>
            <Gauge size={12} />
            {fmtTokens(used)}
            {status.budget > 0 ? ` / ${fmtTokens(status.budget)}` : ''} tokens
          </div>
        )}
        <button className={`side-item settings-btn ${view.kind === 'settings' ? 'active' : ''}`} onClick={() => onView({ kind: 'settings' })}>
          <Settings size={15} strokeWidth={1.8} />
          <span className="side-name">Settings</span>
          {status && !status.has_key && <span className="side-meta warn">Set up a provider</span>}
        </button>
      </div>
    </aside>
  );
}
