import { useEffect, useRef, useState } from 'react';
import { api, errText, Language, PinInput, PinScope, Project, RuleSet, VersionInfo } from '../api';
import { Eye, Trash2, X } from 'lucide-react';
import { useToast } from './Toast';
import { useConfirmRow } from './useConfirmRow';

interface Props {
  project: Project;
  languages: Language[];
  ruleSets: RuleSet[];
  onOpenRuleSet: (id: string) => void;
  tick: number;
  onChanged: () => void;
  onDeleted: () => void;
}

const SCOPES: { value: PinScope; label: string; phrase: string }[] = [
  { value: 'all', label: 'All sections', phrase: 'all sections' },
  { value: 'cad', label: 'CAD geometry only', phrase: 'only the CAD geometry' },
  { value: 'cmf', label: 'Materials and CMF only', phrase: 'only the materials and CMF' },
  { value: 'prompts', label: 'Image-gen prompts only', phrase: 'only the image-gen prompts' },
];

function toInputs(p: Project): PinInput[] {
  return p.pins.map((x) => ({ language_id: x.language_id, version: x.version, role: x.role, scope: x.scope }));
}

/** The pins of a project as text that does not depend on their order, to compare two states. */
const pinsKey = (p: Project) => p.pins.map((x) => `${x.language_id} ${x.version} ${x.role} ${x.scope}`).sort().join('\n');

/** The rule set IDs with one rule set added or removed. */
const withRuleSet = (ids: string[], id: string, on: boolean) => (on ? (ids.includes(id) ? ids : [...ids, id]) : ids.filter((x) => x !== id));

export default function ProjectView({ project, languages, ruleSets, onOpenRuleSet, tick, onChanged, onDeleted }: Props) {
  const toast = useToast();
  const [pins, setPins] = useState<PinInput[]>(toInputs(project));
  const [versions, setVersions] = useState<Record<string, VersionInfo[]>>({});
  const [brief, setBrief] = useState<string | null>(null);
  const [name, setName] = useState(project.name);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const deleteTrigger = useConfirmRow(confirmDelete, () => setConfirmDelete(false));
  // The selected rule sets, with the changes not yet in the reloaded project. The ref has the latest value for the async handlers.
  const [ruleIds, setRuleIdsState] = useState<string[]>(project.rule_set_ids);
  const ruleIdsRef = useRef(ruleIds);
  const setRuleIds = (ids: string[]) => {
    ruleIdsRef.current = ids;
    setRuleIdsState(ids);
  };

  // The library reloads after every queue job and gives a new object each time.
  // Follow the saved pins only when they change, so an edit in progress stays.
  const server = JSON.stringify(toInputs(project));
  useEffect(() => setPins(JSON.parse(server) as PinInput[]), [server]);
  useEffect(() => setName(project.name), [project.name]);
  const serverRuleIds = JSON.stringify(project.rule_set_ids);
  useEffect(() => {
    const ids = JSON.parse(serverRuleIds) as string[];
    ruleIdsRef.current = ids;
    setRuleIdsState(ids);
  }, [serverRuleIds]);

  useEffect(() => {
    let alive = true;
    Promise.all(languages.map((l) => api.listVersions(l.id).then((v) => [l.id, v] as const)))
      .then((pairs) => alive && setVersions(Object.fromEntries(pairs)))
      .catch((e) => toast.error(errText(e)));
    return () => {
      alive = false;
    };
  }, [languages, tick, toast]);

  const title = `“${project.name}”`;

  // A toggle changes only its own rule set, from the latest IDs, so a second toggle before the reload keeps the first.
  // Undo also changes only that rule set, in the project as it is now.
  const toggleRuleSet = async (r: RuleSet, on: boolean) => {
    const ids = withRuleSet(ruleIdsRef.current, r.id, on);
    setRuleIds(ids);
    try {
      await api.setProjectRuleSets(project.id, ids);
      setBrief(null);
      onChanged();
      toast.info(`${title} ${on ? 'now uses' : 'no longer uses'} ${r.name}.`, {
        label: 'Undo',
        run: async () => {
          try {
            const now = (await api.listProjects()).find((p) => p.id === project.id);
            if (!now) {
              toast.error('The project no longer exists. Undo is not possible.');
              return;
            }
            const saved = await api.setProjectRuleSets(project.id, withRuleSet(now.rule_set_ids, r.id, !on));
            setRuleIds(saved.rule_set_ids);
            setBrief(null);
            onChanged();
          } catch (e) {
            toast.error(errText(e));
          }
        },
      });
    } catch (e) {
      setRuleIds(withRuleSet(ruleIdsRef.current, r.id, !on));
      toast.error(errText(e));
    }
  };

  const versioned = languages.filter((l) => (versions[l.id]?.length ?? 0) > 0);
  const primary = pins.find((p) => p.role === 'primary') ?? null;
  const secondaries = pins.filter((p) => p.role === 'secondary');
  const langName = (id: string) => languages.find((l) => l.id === id)?.name ?? id;
  const latestOf = (id: string) => versions[id]?.[0]?.version ?? null;

  /** Undo of a pin change: puts the earlier pins back, but only while the project still has the pins that the change saved. */
  const undoPins = async (before: PinInput[], saved: Project) => {
    try {
      const now = (await api.listProjects()).find((p) => p.id === project.id);
      if (!now) {
        toast.error('The project no longer exists. Undo is not possible.');
        return;
      }
      if (pinsKey(now) !== pinsKey(saved)) {
        toast.error('The pins changed after this. Undo is not possible.');
        return;
      }
      await api.setPins(project.id, before);
      setBrief(null);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  /** Pins apply at once, like "Pin in…" on the Versions tab. The toast can undo the change. */
  const commit = async (next: PinInput[], text: string) => {
    const before = pins;
    setPins(next);
    try {
      const saved = await api.setPins(project.id, next);
      setBrief(null);
      onChanged();
      toast.info(text, { label: 'Undo', run: () => undoPins(before, saved) });
    } catch (e) {
      setPins(before);
      toast.error(errText(e));
    }
  };

  const setPrimary = (languageId: string) => {
    const rest = pins.filter((p) => p.role !== 'primary' && p.language_id !== languageId);
    if (!languageId) return commit(rest, `${title} has no primary language now.`);
    const v = latestOf(languageId);
    if (v === null) return;
    commit([{ language_id: languageId, version: v, role: 'primary', scope: 'all' }, ...rest], `${title} now uses v${v} of ${langName(languageId)} as its primary language.`);
  };

  const update = (pin: PinInput, patch: Partial<PinInput>) => {
    const next = pins.map((p) => (p.language_id === pin.language_id ? { ...p, ...patch } : p));
    const lang = langName(pin.language_id);
    const scope = SCOPES.find((s) => s.value === patch.scope);
    commit(next, scope ? `${title} now takes ${scope.phrase} from ${lang}.` : `${title} now uses v${patch.version} of ${lang}.`);
  };

  const addSecondary = (languageId: string) => {
    const v = latestOf(languageId);
    if (v === null || pins.some((p) => p.language_id === languageId)) return;
    commit([...pins, { language_id: languageId, version: v, role: 'secondary', scope: 'cmf' }], `${title} now takes only the materials and CMF from v${v} of ${langName(languageId)}.`);
  };

  const remove = (pin: PinInput) =>
    commit(
      pins.filter((p) => p.language_id !== pin.language_id),
      `${title} no longer uses ${langName(pin.language_id)}.`,
    );

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

  // A plain function, not a component: a component made in render remounts and loses focus on each change.
  const versionSelect = (pin: PinInput) => (
    <select
      className="select small"
      value={pin.version}
      onChange={(e) => update(pin, { version: Number(e.target.value) })}
      aria-label={`Version of ${langName(pin.language_id)}`}
    >
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
      <button className="btn small" onClick={() => update(pin, { version: l })}>
        Update to v{l}
      </button>
    ) : null;
  };

  return (
    <div className="project">
      <header className="lang-head" data-tauri-drag-region>
        <div className="lang-title">
          <div className="eyebrow">Project</div>
          <input
            className="title-input plain"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={rename}
            onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
            aria-label="Project name"
          />
          <p className="lang-desc muted">
            Agents call <code>get_project_brief</code> with the name “{project.name}”. They get the pinned versions below, not the latest.
          </p>
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
                const on = r.always || ruleIds.includes(r.id);
                return (
                  <div key={r.id} className={`rs-row ${on ? 'on' : ''}`}>
                    <label className="check" style={{ margin: 0 }}>
                      <input type="checkbox" checked={on} disabled={r.always} onChange={(e) => toggleRuleSet(r, e.target.checked)} />
                      <span className="rs-name">{r.name}</span>
                    </label>
                    <span className="muted small">
                      {r.rules.length} rule{r.rules.length === 1 ? '' : 's'}
                      {r.always ? ' · applies to every project' : ''}
                    </span>
                    <div className="spacer" />
                    <button className="link-btn" onClick={() => onOpenRuleSet(r.id)} aria-label={`Open ${r.name}`}>
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
            <select className="select" value={primary?.language_id ?? ''} onChange={(e) => setPrimary(e.target.value)} aria-label="Primary language">
              <option value="">— none —</option>
              {versioned.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}
                </option>
              ))}
            </select>
            {primary && versionSelect(primary)}
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
              {versionSelect(pin)}
              <select
                className="select small"
                value={pin.scope}
                onChange={(e) => update(pin, { scope: e.target.value as PinScope })}
                aria-label={`Scope of ${langName(pin.language_id)}`}
              >
                {SCOPES.map((s) => (
                  <option key={s.value} value={s.value}>
                    {s.label}
                  </option>
                ))}
              </select>
              {stale(pin)}
              <button className="icon-btn" title="Remove" aria-label={`Remove ${langName(pin.language_id)}`} onClick={() => remove(pin)}>
                <X size={14} />
              </button>
            </div>
          ))}
          <div className="pin-row">
            <select className="select small" value="" onChange={(e) => e.target.value && addSecondary(e.target.value)} aria-label="Add a secondary language">
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
              <button className="btn small" autoFocus onClick={() => setConfirmDelete(false)}>
                Cancel
              </button>
              <span className="muted small">Only the project and its pins go. The languages stay.</span>
            </div>
          ) : (
            <button ref={deleteTrigger} className="btn small ghost-danger" onClick={() => setConfirmDelete(true)}>
              <Trash2 size={13} /> Delete project…
            </button>
          )}
        </section>
      </div>
    </div>
  );
}
