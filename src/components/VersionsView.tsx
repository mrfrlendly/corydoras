import { useEffect, useMemo, useState } from 'react';
import { diffLines } from 'diff';
import { api, errText, fmtDate, Language, VersionContent, VersionInfo } from '../api';
import { useToast } from './Toast';

interface Props {
  language: Language;
  tick: number;
  onOpen: (version: number) => void;
}

export default function VersionsView({ language, tick, onOpen }: Props) {
  const toast = useToast();
  const [versions, setVersions] = useState<VersionInfo[]>([]);
  const [a, setA] = useState<number | null>(null);
  const [b, setB] = useState<number | null>(null);
  const [ca, setCa] = useState<VersionContent | null>(null);
  const [cb, setCb] = useState<VersionContent | null>(null);
  const [part, setPart] = useState<'markdown' | 'tokens'>('markdown');

  useEffect(() => {
    api
      .listVersions(language.id)
      .then((vs) => {
        setVersions(vs);
        if (vs.length >= 2) {
          setA((x) => x ?? vs[1].version);
          setB((x) => x ?? vs[0].version);
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

  const chunks = useMemo(() => {
    if (!ca || !cb) return [];
    const left = part === 'markdown' ? ca.markdown : JSON.stringify(ca.tokens, null, 2);
    const right = part === 'markdown' ? cb.markdown : JSON.stringify(cb.tokens, null, 2);
    return diffLines(left, right);
  }, [ca, cb, part]);

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
            <th />
          </tr>
        </thead>
        <tbody>
          {versions.map((v) => (
            <tr key={v.version}>
              <td className="mono">
                v{v.version}
                {v.edited && <span className="tag-pill">edited</span>}
              </td>
              <td>{fmtDate(v.created_at)}</td>
              <td className="mono small">{v.model}</td>
              <td>{v.item_count}</td>
              <td>
                <label className="radio">
                  <input type="radio" name="a" checked={a === v.version} onChange={() => setA(v.version)} /> A
                </label>
                <label className="radio">
                  <input type="radio" name="b" checked={b === v.version} onChange={() => setB(v.version)} /> B
                </label>
              </td>
              <td>
                <button className="link-btn" onClick={() => onOpen(v.version)}>
                  Open
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      {versions.length >= 2 && a !== null && b !== null && (
        <div className="diff">
          <div className="toolbar">
            <span className="hint">
              Changes from v{a} (A) to v{b} (B)
            </span>
            <div className="spacer" />
            <div className="seg">
              <button className={part === 'markdown' ? 'active' : ''} onClick={() => setPart('markdown')}>
                language.md
              </button>
              <button className={part === 'tokens' ? 'active' : ''} onClick={() => setPart('tokens')}>
                tokens.json
              </button>
            </div>
          </div>
          <pre className="diff-body">
            {chunks.map((c, i) => (
              <span key={i} className={c.added ? 'add' : c.removed ? 'del' : 'same'}>
                {c.value}
              </span>
            ))}
          </pre>
        </div>
      )}
    </div>
  );
}
