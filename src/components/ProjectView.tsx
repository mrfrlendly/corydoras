import { useEffect, useState } from 'react';
import { api, errText, Language, PinInput, PinScope, Project, RuleSet, VersionInfo } from '../api';
import { Check, Eye, Trash2, X } from 'lucide-react';
import { useToast } from './Toast';

interface Props {
  project: Project;
  languages: Language[];
  ruleSets: RuleSet[];
  onOpenRuleSet: (id: string) => void;
  tick: number;
  onChanged: () => void;
  onDeleted: () => void;
}

const SCOPES: { value: PinScope; label: string }[] = [
  { value: 'all', label: 'All sections' },
  { value: 'cad', label: 'CAD geometry only' },
  { value: 'cmf', label: 'Materials and CMF only' },
  { value: 'prompts', label: 'Image-gen prompts only' },
];

function toInputs(p: Project): PinInput[] {
  return p.pins.map((x) => ({ language_id: x.language_id, version: x.version, role: x.role, scope: x.scope }));
}

export default function ProjectView({ project, languages, ruleSets, onOpenRuleSet, tick, onChanged, onDeleted }: Props) {
  const toast = useToast();
  const [pins, setPins] = useState<PinInput[]>(toInputs(project));
  const [versions, setVersions] = useState<Record<string, VersionInfo[]>>({});
  const [brief, setBrief] = useState<string | null>(null);
  const [name, setName] = useState(project.name);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => setPins(toInputs(project)), [project]);
  useEffect(() => setName(project.name), [project.name]);

  useEffect(() => {
    let alive = true;
    Promise.all(languages.map((l) => api.listVersions(l.id).then((v) => [l.id, v] as const)))
      .then((pairs) => alive && setVersions(Object.fromEntries(pairs)))
      .catch((e) => toast.error(errText(e)));
    return () => {
      alive = false;
    };
  }, [languages, tick, toast]);

  const toggleRuleSet = async (id: string, on: boolean) => {
    const ids = on ? [...project.rule_set_ids, id] : project.rule_set_ids.filter((x) => x !== id);
    try {
      await api.setProjectRuleSets(project.id, ids);
      setBrief(null);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const versioned = languages.filter((l) => (versions[l.id]?.length ?? 0) > 0);
  const primary = pins.find((p) => p.role === 'primary') ?? null;
  const secondaries = pins.filter((p) => p.role === 'secondary');
  const dirty = JSON.stringify(pins) !== JSON.stringify(toInputs(project));
  const langName = (id: string) => languages.find((l) => l.id === id)?.name ?? id;
  const latestOf = (id: string) => versions[id]?.[0]?.version ?? null;

  const setPrimary = (languageId: string) => {
    const rest = pins.filter((p) => p.role !== 'primary' && p.language_id !== languageId);
    if (!languageId) return setPins(rest);
    const v = latestOf(languageId);
    if (v === null) return;
    setPins([{ language_id: languageId, version: v, role: 'primary', scope: 'all' }, ...rest]);
  };

  const update = (languageId: string, patch: Partial<PinInput>) =>
    setPins((ps) => ps.map((p) => (p.language_id === languageId ? { ...p, ...patch } : p)));

  const addSecondary = (languageId: string) => {
    const v = latestOf(languageId);
    if (v === null || pins.some((p) => p.language_id === languageId)) return;
    setPins([...pins, { language_id: languageId, version: v, role: 'secondary', scope: 'cmf' }]);
  };

  const save = async () => {
    try {
      await api.setPins(project.id, pins);
      toast.info('Pins saved.');
      onChanged();
      setBrief(null);
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const rename = async () => {
    if (name.trim() === project.name) return;
    try {
      await api.renameProject(project.id, name);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
      setName(project.name);
    }
  };

  const VersionSelect = ({ pin }: { pin: PinInput }) => (
    <select className="select small" value={pin.version} onChange={(e) => update(pin.language_id, { version: Number(e.target.value) })}>
      {(versions[pin.language_id] ?? []).map((v) => (
        <option key={v.version} value={v.version}>
          v{v.version}
          {v.version === latestOf(pin.language_id) ? ' (latest)' : ''}
          {v.edited ? ' · edited' : ''}
        </option>
      ))}
    </select>
  );

  const stale = (pin: PinInput) => {
    const l = latestOf(pin.language_id);
    return l !== null && l > pin.version ? (
      <button className="link-btn" onClick={() => update(pin.language_id, { version: l })}>
        Update to v{l}
      </button>
    ) : null;
  };

  return (
    <div className="project">
      <header className="lang-head" data-tauri-drag-region>
        <div className="lang-title">
          <div className="eyebrow">Project</div>
          <input className="title-input plain" value={name} onChange={(e) => setName(e.target.value)} onBlur={rename} onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()} />
          <p className="lang-desc muted">
            Agents call <code>get_project_brief</code> with the name “{project.name}”. They get the pinned versions below, not the latest.
          </p>
        </div>
        <div className="lang-actions">
          {dirty && (
            <>
              <button className="btn small" onClick={() => setPins(toInputs(project))}>
                Discard
              </button>
              <button className="btn small primary" onClick={save}>
                <Check size={13} /> Save pins
              </button>
            </>
          )}
        </div>
      </header>

      <div className="project-body">
        {versioned.length === 0 && (
          <div className="notice">No language has a version yet. A language needs at least one version before a project can pin it.</div>
        )}

        <section className="panel">
          <h3>Hard rule sets</h3>
          {ruleSets.length === 0 ? (
            <p className="muted small">No rule sets yet. Click + next to Rule sets in the sidebar.</p>
          ) : (
            <div className="rs-list">
              {ruleSets.map((r) => {
                const on = r.always || project.rule_set_ids.includes(r.id);
                return (
                  <div key={r.id} className={`rs-row ${on ? 'on' : ''}`}>
                    <label className="check" style={{ margin: 0 }}>
                      <input type="checkbox" checked={on} disabled={r.always} onChange={(e) => toggleRuleSet(r.id, e.target.checked)} />
                      <span className="rs-name">{r.name}</span>
                    </label>
                    <span className="muted small">
                      {r.rules.length} rule{r.rules.length === 1 ? '' : 's'}
                      {r.always ? ' · applies to every project' : ''}
                    </span>
                    <div className="spacer" />
                    <button className="link-btn" onClick={() => onOpenRuleSet(r.id)}>
                      Open
                    </button>
                  </div>
                );
              })}
            </div>
          )}
          <p className="muted small">
            Hard rules win over every language rule. They also supply the values that a language marks process-dependent, such as wall
            thickness and clearances.
          </p>
        </section>

        <section className="panel">
          <h3>Primary language</h3>
          <div className="pin-row">
            <select className="select" value={primary?.language_id ?? ''} onChange={(e) => setPrimary(e.target.value)}>
              <option value="">— none —</option>
              {versioned.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}
                </option>
              ))}
            </select>
            {primary && <VersionSelect pin={primary} />}
            {primary && stale(primary)}
          </div>
          <p className="muted small">The primary language applies to the whole design. Its rules win in a conflict.</p>
        </section>

        <section className="panel">
          <h3>Secondary languages</h3>
          {secondaries.length === 0 && <p className="muted small">None. Add one to borrow only part of another language, for example its CMF.</p>}
          {secondaries.map((pin) => (
            <div key={pin.language_id} className="pin-row">
              <span className="pin-name">{langName(pin.language_id)}</span>
              <VersionSelect pin={pin} />
              <select className="select small" value={pin.scope} onChange={(e) => update(pin.language_id, { scope: e.target.value as PinScope })}>
                {SCOPES.map((s) => (
                  <option key={s.value} value={s.value}>
                    {s.label}
                  </option>
                ))}
              </select>
              {stale(pin)}
              <button className="icon-btn" title="Remove" onClick={() => setPins(pins.filter((p) => p.language_id !== pin.language_id))}>
                <X size={14} />
              </button>
            </div>
          ))}
          <div className="pin-row">
            <select className="select small" value="" onChange={(e) => e.target.value && addSecondary(e.target.value)}>
              <option value="">+ Add a secondary language…</option>
              {versioned
                .filter((l) => !pins.some((p) => p.language_id === l.id))
                .map((l) => (
                  <option key={l.id} value={l.id}>
                    {l.name}
                  </option>
                ))}
            </select>
          </div>
        </section>

        <section className="panel">
          <div className="row">
            <h3 className="grow">Brief preview</h3>
            <button
              className="btn small"
              disabled={dirty}
              title={dirty ? 'Save the pins first' : ''}
              onClick={() =>
                api
                  .projectBrief(project.id)
                  .then(setBrief)
                  .catch((e) => toast.error(errText(e)))
              }
            >
              <Eye size={13} /> Show what agents get
            </button>
          </div>
          {brief && <pre className="json brief">{brief}</pre>}
        </section>

        <section className="panel danger-zone">
          {confirmDelete ? (
            <div className="row">
              <button
                className="btn danger small"
                onClick={() =>
                  api
                    .deleteProject(project.id)
                    .then(onDeleted)
                    .catch((e) => toast.error(errText(e)))
                }
              >
                <Trash2 size={13} /> Delete project
              </button>
              <button className="btn small" onClick={() => setConfirmDelete(false)}>
                Cancel
              </button>
              <span className="muted small">Only the project and its pins go. The languages stay.</span>
            </div>
          ) : (
            <button className="btn small ghost-danger" onClick={() => setConfirmDelete(true)}>
              <Trash2 size={13} /> Delete project…
            </button>
          )}
        </section>
      </div>
    </div>
  );
}
