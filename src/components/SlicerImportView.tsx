import { useEffect, useMemo, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { Check, FileUp, Printer, TriangleAlert } from 'lucide-react';
import { api, CompareRow, errText, fmtMeasure, ProfileRef, RuleSet, SlicerDerived, SlicerInput } from '../api';
import { useToast } from './Toast';

interface Props {
  onCreated: (r: RuleSet) => void;
  onCancel: () => void;
}

type Kind = ProfileRef['kind'];

const KIND_LABEL: Record<Kind, string> = { process: 'Process profile', machine: 'Printer profile', filament: 'Filament profile' };
const KIND_HINT: Record<Kind, string> = {
  process: 'Layer height, line widths, wall loops and the support angle. Necessary.',
  machine: 'The nozzle and the build volume. Optional.',
  filament: 'The material and its temperatures. Optional.',
};
const STATUS_LABEL: Record<CompareRow['status'], string> = {
  same: 'same',
  differs: 'differs',
  new: 'not in the built-in set',
  'builtin-only': 'only in the built-in set',
};

function baseName(path: string) {
  return path.split('/').pop() ?? path;
}

/** One profile: a filter, the list of installed profiles, and a file dialog for a profile from elsewhere. */
function ProfilePick({ kind, profiles, value, onValue }: { kind: Kind; profiles: ProfileRef[]; value: string | null; onValue: (path: string | null) => void }) {
  const [filter, setFilter] = useState('');
  const mine = useMemo(() => profiles.filter((p) => p.kind === kind), [profiles, kind]);
  const shown = useMemo(() => {
    const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
    return mine.filter((p) => words.every((w) => p.name.toLowerCase().includes(w))).slice(0, 400);
  }, [mine, filter]);
  const groups = useMemo(() => {
    const out: { label: string; items: ProfileRef[] }[] = [];
    for (const p of shown) {
      const label = `${p.app} · ${p.user ? 'your profiles' : 'built-in'}`;
      const g = out.find((x) => x.label === label);
      if (g) g.items.push(p);
      else out.push({ label, items: [p] });
    }
    return out;
  }, [shown]);
  const listed = value !== null && shown.some((p) => p.path === value);

  const choose = async () => {
    const picked = await open({ multiple: false, title: `Choose a ${KIND_LABEL[kind].toLowerCase()}`, filters: [{ name: 'Slicer profile', extensions: ['json'] }] });
    if (typeof picked === 'string') onValue(picked);
  };

  return (
    <div className="field slicer-pick">
      <span>
        {KIND_LABEL[kind]} <span className="muted">· {KIND_HINT[kind]}</span>
      </span>
      <div className="row nowrap" style={{ marginTop: 0 }}>
        <input className="small-input" placeholder={`Filter ${mine.length} profiles`} value={filter} onChange={(e) => setFilter(e.target.value)} />
        <select className="select grow-input" value={listed ? value! : ''} onChange={(e) => onValue(e.target.value || null)} aria-label={KIND_LABEL[kind]}>
          <option value="">{kind === 'process' ? 'Choose a profile…' : 'None'}</option>
          {groups.map((g) => (
            <optgroup key={g.label} label={g.label}>
              {g.items.map((p) => (
                <option key={p.path} value={p.path}>
                  {p.name}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
        <button className="btn small" onClick={choose} title="A profile file from another folder">
          <FileUp size={13} /> File…
        </button>
      </div>
      {value !== null && !listed && (
        <div className="small mono wrap">
          {baseName(value)}{' '}
          <button className="link-btn" onClick={() => onValue(null)}>
            Remove
          </button>
        </div>
      )}
    </div>
  );
}

export default function SlicerImportView({ onCreated, onCancel }: Props) {
  const toast = useToast();
  const [profiles, setProfiles] = useState<ProfileRef[] | null>(null);
  const [input, setInput] = useState<SlicerInput>({ process: null, machine: null, filament: null, project: null });
  const [derived, setDerived] = useState<SlicerDerived | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [fill, setFill] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .slicerProfiles()
      .then(setProfiles)
      .catch((e) => {
        setProfiles([]);
        toast.error(errText(e));
      });
  }, [toast]);

  // Work the rules out again after each choice.
  useEffect(() => {
    if (!input.process && !input.project) {
      setDerived(null);
      setError(null);
      return;
    }
    let alive = true;
    api
      .slicerPreview(input)
      .then((d) => {
        if (!alive) return;
        setDerived(d);
        setError(null);
        setName(d.name);
      })
      .catch((e) => {
        if (!alive) return;
        setDerived(null);
        setError(errText(e));
      });
    return () => {
      alive = false;
    };
  }, [input]);

  const chooseProject = async () => {
    const picked = await open({ multiple: false, title: 'Choose a project file of Bambu Studio or OrcaSlicer', filters: [{ name: 'Project file', extensions: ['3mf'] }] });
    if (typeof picked === 'string') setInput({ process: null, machine: null, filament: null, project: picked });
  };

  const create = async () => {
    setBusy(true);
    try {
      const r = await api.slicerImport(input, name, fill);
      toast.info(`Rule set “${r.name}” made with ${r.rules.length} rules. Select it in a project to use it.`);
      onCreated(r);
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const differs = derived ? derived.compare.filter((c) => c.status === 'differs').length : 0;

  return (
    <div className="settings">
      <header className="lang-head" data-tauri-drag-region>
        <div className="lang-title">
          <div className="eyebrow">Hard rule set</div>
          <h1>Import a slicer profile</h1>
          <p className="lang-desc muted">
            Makes a rule set from a Bambu Studio or OrcaSlicer profile, and shows where its values differ from the built-in “FDM · PETG · Bambu P1S” set.
          </p>
        </div>
        <div className="lang-actions">
          <button className="btn small" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn small primary" onClick={create} disabled={!derived || busy || !name.trim()}>
            <Check size={13} /> Make rule set
          </button>
        </div>
      </header>

      <div className="settings-body">
        <section className="panel">
          <h3>
            <Printer size={15} /> Profiles
          </h3>
          {profiles === null ? (
            <p className="muted small">Looking for the profiles of Bambu Studio and OrcaSlicer…</p>
          ) : profiles.length === 0 ? (
            <p className="muted small">No profiles of Bambu Studio or OrcaSlicer on this Mac. Choose a profile file or a project file.</p>
          ) : null}
          {input.project ? (
            <div className="lib-path">
              <span className="mono small wrap">{input.project}</span>
              <button className="btn small" onClick={() => setInput({ process: null, machine: null, filament: null, project: null })}>
                Use profiles instead
              </button>
            </div>
          ) : (
            <>
              {(['process', 'machine', 'filament'] as Kind[]).map((k) => (
                <ProfilePick key={k} kind={k} profiles={profiles ?? []} value={input[k]} onValue={(path) => setInput((i) => ({ ...i, [k]: path }))} />
              ))}
              <div className="row">
                <button className="btn small" onClick={chooseProject} title="A project file holds the printer, the filament and the process settings in one place">
                  <FileUp size={13} /> From a project file (.3mf)…
                </button>
                <span className="hint">A project that you saved in the slicer holds all three profiles with every value.</span>
              </div>
            </>
          )}
          {error && <div className="error-box">{error}</div>}
        </section>

        {derived && (
          <>
            <section className="panel">
              <h3>Rule set</h3>
              <label className="field">
                <span>Name</span>
                <input value={name} onChange={(e) => setName(e.target.value)} />
              </label>
              {derived.notes.map((n, i) => (
                <div key={i} className="warn-box">
                  <TriangleAlert size={14} /> {n}
                </div>
              ))}
              <table className="vtable">
                <thead>
                  <tr>
                    <th>Rule</th>
                    <th>From the profile</th>
                    <th>How it follows from the profile</th>
                  </tr>
                </thead>
                <tbody>
                  {derived.rules.map((r) => (
                    <tr key={r.name}>
                      <td>{r.name}</td>
                      <td className="mono">{r.value !== null ? fmtMeasure(r.value, r.unit) : '—'}</td>
                      <td className="small">{r.note}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <label className="check">
                <input type="checkbox" checked={fill} onChange={(e) => setFill(e.target.checked)} />
                Also copy the built-in rules that a slicer profile cannot give (clearances, hole oversize, the instructions)
              </label>
            </section>

            <section className="panel">
              <h3>Against the built-in set</h3>
              <p className="muted small">
                {differs === 0
                  ? 'No value differs from “FDM · PETG · Bambu P1S”.'
                  : `${differs} value${differs === 1 ? '' : 's'} differ${differs === 1 ? 's' : ''} from “FDM · PETG · Bambu P1S”.`}{' '}
                The built-in values are common starting points; the profile says what this printer and this setup do.
              </p>
              <table className="vtable">
                <thead>
                  <tr>
                    <th>Rule</th>
                    <th>Profile</th>
                    <th>Built-in</th>
                    <th>Difference</th>
                  </tr>
                </thead>
                <tbody>
                  {derived.compare.map((c) => (
                    <tr key={c.rule} className={`cmp-${c.status}`}>
                      <td>{c.rule}</td>
                      <td className="mono">{c.imported !== null ? fmtMeasure(c.imported, c.unit) : '—'}</td>
                      <td className="mono">{c.builtin !== null ? fmtMeasure(c.builtin, c.unit) : '—'}</td>
                      <td className="small">
                        {c.status === 'differs' && c.imported !== null && c.builtin !== null ? (
                          <span className="cmp-delta">
                            {c.imported > c.builtin ? '+' : '−'}
                            {fmtMeasure(Math.abs(c.imported - c.builtin), c.unit)}
                          </span>
                        ) : (
                          <span className="muted">{STATUS_LABEL[c.status]}</span>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          </>
        )}
      </div>
    </div>
  );
}
