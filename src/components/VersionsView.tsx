import { useEffect, useMemo, useRef, useState } from 'react';
import { diffLines } from 'diff';
import { Pin, Undo2 } from 'lucide-react';
import { api, DiffChange, errText, fmtDate, Language, PinInput, Project, VersionContent, VersionDiff, VersionInfo } from '../api';
import { useToast } from './Toast';

interface Props {
  language: Language;
  projects: Project[];
  tick: number;
  onOpen: (version: number) => void;
  onChanged: () => void;
}

type Part = 'changes' | 'markdown' | 'tokens';

/** The changes of a semantic diff, one group for each section of the document. */
function ChangeList({ diff }: { diff: VersionDiff }) {
  const groups = useMemo(() => {
    const out: { section: string; changes: DiffChange[] }[] = [];
    for (const c of diff.changes) {
      const last = out[out.length - 1];
      if (last && last.section === c.section) last.changes.push(c);
      else out.push({ section: c.section, changes: [c] });
    }
    return out;
  }, [diff]);
  if (!diff.structured) {
    return (
      <div className="notice">
        One of these versions uses the older format, which has no families and measures to compare. Use <b>language.md</b> or <b>tokens.json</b> to see the
        text changes.
      </div>
    );
  }
  if (diff.changes.length === 0) return <div className="notice">These two versions have the same content.</div>;
  return (
    <div className="changes">
      {groups.map((g) => (
        <section key={g.section}>
          <h4>{g.section}</h4>
          <ul>
            {g.changes.map((c, i) => (
              <li key={i} className={`change change-${c.kind}`}>
                <span className="change-kind">{c.kind}</span>
                <span>{c.text}</span>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

export default function VersionsView({ language, projects, tick, onOpen, onChanged }: Props) {
  const toast = useToast();
  const [versions, setVersions] = useState<VersionInfo[]>([]);
  const [a, setA] = useState<number | null>(null);
  const [b, setB] = useState<number | null>(null);
  const [ca, setCa] = useState<VersionContent | null>(null);
  const [cb, setCb] = useState<VersionContent | null>(null);
  const [diff, setDiff] = useState<VersionDiff | null>(null);
  const [part, setPart] = useState<Part>('changes');
  const [confirmRestore, setConfirmRestore] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const diffRef = useRef<HTMLDivElement>(null);
  // The version whose Restore button gets the focus back after Cancel.
  const refocusRestore = useRef<number | null>(null);

  const latest = versions[0]?.version ?? null;

  // With many versions the comparison is below the table: bring it into view when A or B changes.
  const showDiff = () =>
    requestAnimationFrame(() => {
      const el = diffRef.current;
      const pane = el?.closest('.versions');
      if (!el || !pane) return;
      if (el.getBoundingClientRect().top > pane.getBoundingClientRect().bottom - 160) el.scrollIntoView({ block: 'nearest' });
    });

  const cancelRestore = () => {
    refocusRestore.current = confirmRestore;
    setConfirmRestore(null);
  };

  useEffect(() => {
    api
      .listVersions(language.id)
      .then((vs) => {
        setVersions(vs);
        if (vs.length >= 2) {
          // After a new version, compare it with the one before.
          setA((x) => (x === null || !vs.some((v) => v.version === x) ? vs[1].version : x));
          setB((x) => (x === null || !vs.some((v) => v.version === x) ? vs[0].version : x));
        }
      })
      .catch((e) => toast.error(errText(e)));
  }, [language.id, tick, toast]);

  useEffect(() => {
    if (a === null) return setCa(null);
    api.getVersion(language.id, a).then(setCa).catch((e) => toast.error(errText(e)));
  }, [language.id, a, toast]);

  useEffect(() => {
    if (b === null) return setCb(null);
    api.getVersion(language.id, b).then(setCb).catch((e) => toast.error(errText(e)));
  }, [language.id, b, toast]);

  useEffect(() => {
    if (a === null || b === null) return setDiff(null);
    let alive = true;
    api
      .versionDiff(language.id, a, b)
      .then((d) => alive && setDiff(d))
      .catch((e) => toast.error(errText(e)));
    return () => {
      alive = false;
    };
  }, [language.id, a, b, toast]);

  const chunks = useMemo(() => {
    if (!ca || !cb || part === 'changes') return [];
    const left = part === 'markdown' ? ca.markdown : JSON.stringify(ca.tokens, null, 2);
    const right = part === 'markdown' ? cb.markdown : JSON.stringify(cb.tokens, null, 2);
    return diffLines(left, right);
  }, [ca, cb, part]);

  const restore = async (v: number) => {
    setBusy(true);
    try {
      const made = await api.restoreVersion(language.id, v);
      toast.info(`Version ${v} is back as version ${made}. The versions in between stay in the list.`);
      setConfirmRestore(null);
      setA(v);
      setB(made);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setBusy(false);
    }
  };

  // Pins a version in a project: changes the pin of this language, or adds one.
  const pin = async (projectId: string, v: number) => {
    const project = projects.find((p) => p.id === projectId);
    if (!project) return;
    const before: PinInput[] = project.pins.map((x) => ({ language_id: x.language_id, version: x.version, role: x.role, scope: x.scope }));
    const pins: PinInput[] = before.map((x) => ({ ...x }));
    const mine = pins.find((x) => x.language_id === language.id);
    if (mine) mine.version = v;
    else pins.push({ language_id: language.id, version: v, role: pins.some((x) => x.role === 'primary') ? 'secondary' : 'primary', scope: 'all' });
    try {
      await api.setPins(projectId, pins);
      toast.info(
        mine
          ? `“${project.name}” now uses v${v} of ${language.name}.`
          : `v${v} of ${language.name} is pinned in “${project.name}” as a ${pins[pins.length - 1].role} language.`,
        {
          label: 'Undo',
          run: () =>
            api
              .setPins(projectId, before)
              .then(onChanged)
              .catch((e) => toast.error(errText(e))),
        },
      );
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  if (versions.length === 0) {
    return (
      <div className="doc-empty">
        <h2>No versions yet</h2>
        <p>Each synthesis pass and each edit that you save makes a new, immutable version. They appear here.</p>
      </div>
    );
  }

  return (
    <div className="versions">
      <table className="vtable">
        <thead>
          <tr>
            <th>Version</th>
            <th>Date</th>
            <th>Model</th>
            <th>Items</th>
            <th>Compare</th>
            <th>Used by</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {versions.map((v) => {
            const users = projects.filter((p) => p.pins.some((x) => x.language_id === language.id && x.version === v.version));
            return (
              <tr key={v.version}>
                <td className="mono">
                  v{v.version}
                  {v.edited && <span className="tag-pill">edited</span>}
                  {v.version === latest && <span className="tag-pill">latest</span>}
                </td>
                <td>{fmtDate(v.created_at)}</td>
                <td className="mono small">{v.model}</td>
                <td>{v.item_count}</td>
                <td>
                  <label className="radio">
                    <input
                      type="radio"
                      name="a"
                      checked={a === v.version}
                      onChange={() => {
                        setA(v.version);
                        showDiff();
                      }}
                    />{' '}
                    A
                  </label>
                  <label className="radio">
                    <input
                      type="radio"
                      name="b"
                      checked={b === v.version}
                      onChange={() => {
                        setB(v.version);
                        showDiff();
                      }}
                    />{' '}
                    B
                  </label>
                </td>
                <td className="small">{users.length > 0 ? users.map((p) => p.name).join(', ') : <span className="muted">—</span>}</td>
                <td>
                  <div className="version-tools">
                    <button className="link-btn" onClick={() => onOpen(v.version)}>
                      Open
                    </button>
                    {projects.length > 0 && (
                      <select
                        className="select small"
                        value=""
                        onChange={(e) => e.target.value && pin(e.target.value, v.version)}
                        title="Make a project use this version"
                        aria-label={`Pin version ${v.version} in a project`}
                      >
                        <option value="">Pin in…</option>
                        {projects.map((p) => (
                          <option key={p.id} value={p.id}>
                            {p.name}
                          </option>
                        ))}
                      </select>
                    )}
                    {v.version !== latest &&
                      (confirmRestore === v.version ? (
                        <span className="version-confirm" onKeyDown={(e) => e.key === 'Escape' && !busy && cancelRestore()}>
                          <button className="btn small primary" disabled={busy} onClick={() => restore(v.version)}>
                            Restore as v{(latest ?? 0) + 1}
                          </button>
                          <button className="btn small" disabled={busy} onClick={cancelRestore} autoFocus>
                            Cancel
                          </button>
                        </span>
                      ) : (
                        <button
                          className="btn small"
                          onClick={() => setConfirmRestore(v.version)}
                          title="Roll back: save this version again as the newest one. Agents without a project then get it."
                          ref={(el) => {
                            if (el && refocusRestore.current === v.version) {
                              refocusRestore.current = null;
                              el.focus();
                            }
                          }}
                        >
                          <Undo2 size={13} /> Restore
                        </button>
                      ))}
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {projects.length > 0 && (
        <p className="muted small">
          <Pin size={12} /> A project keeps the version that it pins, also when the language gets newer versions. <b>Restore</b> rolls the language itself
          back, for agents that read it without a project.
        </p>
      )}

      {versions.length >= 2 && a !== null && b !== null && (
        <div className="diff" ref={diffRef}>
          <div className="toolbar">
            <span className="hint">
              {part === 'changes' && diff && diff.structured
                ? `${diff.changes.length} change${diff.changes.length === 1 ? '' : 's'} from v${a} (A) to v${b} (B)`
                : `Changes from v${a} (A) to v${b} (B)`}
            </span>
            <div className="spacer" />
            <div className="seg" role="group" aria-label="Show">
              <button className={part === 'changes' ? 'active' : ''} aria-pressed={part === 'changes'} onClick={() => setPart('changes')}>
                Changes
              </button>
              <button className={part === 'markdown' ? 'active' : ''} aria-pressed={part === 'markdown'} onClick={() => setPart('markdown')}>
                language.md
              </button>
              <button className={part === 'tokens' ? 'active' : ''} aria-pressed={part === 'tokens'} onClick={() => setPart('tokens')}>
                tokens.json
              </button>
            </div>
          </div>
          {part === 'changes' ? (
            diff && <ChangeList diff={diff} />
          ) : (
            <pre className="diff-body">
              {chunks.map((c, i) => (
                <span key={i} className={c.added ? 'add' : c.removed ? 'del' : 'same'}>
                  {c.value}
                </span>
              ))}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
