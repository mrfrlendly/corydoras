import { useEffect, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { save as saveDialog } from '@tauri-apps/plugin-dialog';
import { api, errText, EXPORT_FORMATS, ExportFormat, fmtDate, Inheritance, Language, VersionContent, VersionInfo } from '../api';
import { Check, Copy, GitBranch, Pencil, X } from 'lucide-react';
import { useToast } from './Toast';
import ProposalsPanel from './ProposalsPanel';

interface Props {
  language: Language;
  version: number | null;
  tick: number;
  /** Open amendment proposals (from the status poll). */
  openProposals: number;
  onVersion: (v: number | null) => void;
  onChanged: () => void;
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = document.createElement('textarea');
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand('copy');
    ta.remove();
    return ok;
  }
}

function Swatch({ c }: { c: Record<string, unknown> }) {
  const hex = typeof c.hex === 'string' ? c.hex : '#888888';
  return (
    <div className="cmf-card">
      <div className="cmf-color" style={{ background: hex }} />
      <div className="cmf-text">
        <div className="cmf-role">
          {String(c.role ?? '')}
          {c.scope && c.scope !== 'global' ? <span className="muted"> · {String(c.scope)}</span> : null}
        </div>
        <div className="mono">{hex}</div>
        {c.ral ? <div className="muted">{String(c.ral)}</div> : null}
        {c.pantone_approx ? <div className="muted">≈ {String(c.pantone_approx)}</div> : null}
        <div className="muted">{[c.material, c.finish].filter(Boolean).map(String).join(' · ')}</div>
        {c.note ? <div className="muted">{String(c.note)}</div> : null}
      </div>
    </div>
  );
}

function CmfSwatches({ tokens }: { tokens: unknown }) {
  const cmf = (tokens as { cmf?: unknown })?.cmf;
  // Schema 1: an array. Schema 2: { object: [], presentation: [] }.
  const objects = Array.isArray(cmf) ? cmf : ((cmf as { object?: unknown })?.object as unknown[]) ?? [];
  const presentation = Array.isArray(cmf) ? [] : (((cmf as { presentation?: unknown })?.presentation as unknown[]) ?? []);
  if (objects.length === 0 && presentation.length === 0) return null;
  return (
    <>
      <h3>Product CMF</h3>
      <div className="cmf">
        {(objects as Record<string, unknown>[]).map((c, i) => (
          <Swatch key={i} c={c} />
        ))}
      </div>
      {presentation.length > 0 && (
        <>
          <h3>Presentation only</h3>
          <div className="cmf">
            {(presentation as Record<string, unknown>[]).map((c, i) => (
              <Swatch key={i} c={c} />
            ))}
          </div>
        </>
      )}
    </>
  );
}

function showValue(v: unknown): string {
  if (v === null || v === undefined) return 'nothing';
  const s = typeof v === 'string' ? `“${v}”` : JSON.stringify(v);
  return s.length > 160 ? `${s.slice(0, 160)}…` : s;
}

/** Two values side by side. Two long texts start where they differ, so the change is in view. */
function showChange(before: unknown, after: unknown): string {
  if (typeof before === 'string' && typeof after === 'string' && (before.length > 150 || after.length > 150)) {
    let same = 0;
    while (same < before.length && same < after.length && before[same] === after[same]) same++;
    const from = Math.max(0, same - 40);
    const cut = (t: string) => `“${from > 0 ? '…' : ''}${t.slice(from, from + 150)}${t.length > from + 150 ? '…' : ''}”`;
    return `${cut(before)} → ${cut(after)}`;
  }
  return `${showValue(before)} → ${showValue(after)}`;
}

/** For a language that extends a base: the base, and each change that this language makes to it. */
function InheritancePanel({ language, tick, onChanged }: { language: Language; tick: number; onChanged: () => void }) {
  const toast = useToast();
  const [inh, setInh] = useState<Inheritance | null>(null);
  useEffect(() => {
    let alive = true;
    api
      .inheritance(language.id)
      .then((i) => alive && setInh(i))
      .catch((e) => toast.error(errText(e)));
    return () => {
      alive = false;
    };
  }, [language.id, language.base_id, language.override_count, tick, toast]);
  if (!inh) return null;
  const behind = inh.pinned_version !== null && inh.base_latest !== null && inh.base_latest > inh.pinned_version;
  return (
    <div className="inherit">
      <div className="inherit-head">
        <GitBranch size={14} />
        <span>
          Extends <b>{inh.base_name}</b>
          {inh.built_from !== null ? ` v${inh.built_from}` : ''}
          {inh.pinned_version === null ? ' (follows the latest version)' : ' (pinned)'} · {inh.overrides.length} change{inh.overrides.length === 1 ? '' : 's'}
        </span>
        {behind && (
          <button
            className="link-btn"
            onClick={() =>
              api
                .setBase(language.id, inh.base_id, inh.base_latest)
                .then(onChanged)
                .catch((e) => toast.error(errText(e)))
            }
          >
            Update to v{inh.base_latest}
          </button>
        )}
      </div>
      {inh.error && <div className="error-box">{inh.error}</div>}
      {inh.overrides.length === 0 ? (
        <div className="small muted">
          No changes yet: this document is the same as the base. Click <b>Edit</b> and change tokens.json; the difference to the base becomes the changes of
          this language, and they stay when the base gets a new version.
        </div>
      ) : (
        <ul className="overrides">
          {inh.overrides.map((o, i) => (
            <li key={i} className={o.problem ? 'override bad' : 'override'}>
              <span className="change-kind">{o.op === 'replace' ? 'changed' : o.op === 'add' ? 'added' : 'removed'}</span>
              <span className="override-body">
                <code className="mono">{o.path}</code>{' '}
                {o.op === 'replace' && showChange(o.before, o.after)}
                {o.op === 'add' && showValue(o.after)}
                {o.op === 'remove' && <span className="muted">the base has {showValue(o.before)}</span>}
                {o.problem && <span className="error-text"> · Does not fit the base now: {o.problem}</span>}
              </span>
              <button
                className="icon-btn"
                title="Remove this change: the value of the base comes back"
                aria-label="Remove this change"
                onClick={() =>
                  api
                    .removeOverride(language.id, i)
                    .then(() => {
                      toast.info('Change removed. The value of the base applies again.');
                      onChanged();
                    })
                    .catch((e) => toast.error(errText(e)))
                }
              >
                <X size={13} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export default function DocumentView({ language, version, tick, openProposals, onVersion, onChanged }: Props) {
  const toast = useToast();
  const [versions, setVersions] = useState<VersionInfo[]>([]);
  const [content, setContent] = useState<VersionContent | null>(null);
  const [editing, setEditing] = useState(false);
  const [md, setMd] = useState('');
  const [tokens, setTokens] = useState('');
  const [saving, setSaving] = useState(false);

  const latest = versions[0]?.version ?? null;
  const shown = version ?? latest;

  useEffect(() => {
    api
      .listVersions(language.id)
      .then(setVersions)
      .catch((e) => toast.error(errText(e)));
  }, [language.id, tick, toast]);

  useEffect(() => {
    if (shown === null) {
      setContent(null);
      return;
    }
    api
      .getVersion(language.id, shown)
      .then(setContent)
      .catch((e) => toast.error(errText(e)));
  }, [language.id, shown, toast]);

  if (shown === null) {
    return (
      <div className="doc-empty">
        <h2>No version yet</h2>
        <p>
          Add images on the Board. Each image gets analyzed automatically. The first language document appears after the analysis
          finishes and no item changes for the debounce time (default 60 s). Click <b>Synthesize now</b> to start it at once.
        </p>
      </div>
    );
  }

  const startEdit = () => {
    if (!content) return;
    setMd(content.markdown);
    setTokens(JSON.stringify(content.tokens, null, 2));
    setEditing(true);
  };

  const save = async () => {
    setSaving(true);
    try {
      const v = await api.saveEditedVersion(language.id, md, tokens);
      setEditing(false);
      onVersion(null);
      toast.info(`Saved as version ${v}.`);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setSaving(false);
    }
  };

  const exportAs = async (format: ExportFormat) => {
    try {
      const x = await api.exportLanguage(language.id, shown, format, null);
      const path = await saveDialog({ title: 'Export the language', defaultPath: x.file_name });
      if (typeof path !== 'string') return;
      await api.saveTextFile(path, x.content);
      toast.info(`Exported v${shown} to ${path}.`);
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const copyForAgent = async () => {
    try {
      const text = await api.languageText(language.id, shown);
      toast.info((await copyText(text)) ? 'Copied the document and tokens.json.' : 'Copy failed.');
    } catch (e) {
      toast.error(errText(e));
    }
  };

  return (
    <div className="doc">
      <div className="toolbar">
        <select className="select small" value={shown} onChange={(e) => onVersion(Number(e.target.value) === latest ? null : Number(e.target.value))}>
          {versions.map((v) => (
            <option key={v.version} value={v.version}>
              v{v.version}
              {v.version === latest ? ' (latest)' : ''}
              {v.edited ? ' · edited' : ''}
            </option>
          ))}
        </select>
        {content && (
          <span className="hint">
            {fmtDate(content.info.created_at)} · {content.info.model} · {content.info.item_count} items
            {content.info.schema < 2 ? ' · older format' : ''}
          </span>
        )}
        {content && content.info.warnings.length > 0 && (
          <span className="chip chip-warn warn-chip" title={content.info.warnings.join('\n')}>
            {content.info.warnings.length} review note{content.info.warnings.length === 1 ? '' : 's'}
          </span>
        )}
        <div className="spacer" />
        {!editing && (
          <>
            {content && content.info.schema >= 2 && (
              <select
                className="select small"
                value=""
                onChange={(e) => e.target.value && exportAs(e.target.value as ExportFormat)}
                title="Write the measures and colours of this version to a file"
                aria-label="Export this version"
              >
                <option value="">Export…</option>
                {EXPORT_FORMATS.map((f) => (
                  <option key={f.id} value={f.id}>
                    {f.label}
                  </option>
                ))}
              </select>
            )}
            <button className="btn small" onClick={copyForAgent} title="Copy the text that agents get">
              <Copy size={13} /> Copy for agent
            </button>
            <button className="btn small" onClick={startEdit} disabled={!content}>
              <Pencil size={13} /> Edit
            </button>
          </>
        )}
        {editing && (
          <>
            <button className="btn small" onClick={() => setEditing(false)} disabled={saving}>
              Cancel
            </button>
            <button className="btn small primary" onClick={save} disabled={saving}>
              <Check size={13} /> Save as new version
            </button>
          </>
        )}
      </div>

      {version !== null && version !== latest && !editing && (
        <div className="notice">
          You see an older version. <button className="link-btn" onClick={() => onVersion(null)}>Show the latest (v{latest})</button>
        </div>
      )}

      {!editing && language.base_id && (
        <InheritancePanel
          language={language}
          tick={tick}
          onChanged={() => {
            onVersion(null);
            onChanged();
          }}
        />
      )}

      {!editing && (
        <ProposalsPanel
          language={language}
          openCount={openProposals}
          tick={tick}
          onAccepted={() => {
            onVersion(null);
            onChanged();
          }}
        />
      )}

      {content && content.info.schema < 2 && !editing && (
        <div className="notice">
          This version uses the older format (one global rule set). Click <b>Synthesize now</b> to make a version with object families,
          measures with evidence, and exceptions. For the best result, click <b>Analyze all again</b> first.
        </div>
      )}

      {editing ? (
        content && content.info.schema >= 2 ? (
          <div className="doc-edit single">
            <label className="field">
              <span>tokens.json — the app renders language.md from it</span>
              <textarea className="mono-area" value={tokens} onChange={(e) => setTokens(e.target.value)} spellCheck={false} />
            </label>
            <p className="muted small">
              {language.base_id
                ? 'This language extends a base. Your edit becomes a new version, and the difference to the base is kept as the changes of this language: they stay when the base gets a new version.'
                : 'Your edit becomes a new version after the same validation that the model output gets. Later automatic versions treat your edit as authoritative where the references still support it.'}
            </p>
          </div>
        ) : (
          <div className="doc-edit">
            <label className="field">
              <span>language.md</span>
              <textarea className="mono-area" value={md} onChange={(e) => setMd(e.target.value)} spellCheck={false} />
            </label>
            <label className="field">
              <span>tokens.json</span>
              <textarea className="mono-area" value={tokens} onChange={(e) => setTokens(e.target.value)} spellCheck={false} />
            </label>
            <p className="muted small">
              Your edit becomes a new version. Later automatic versions treat your edit as authoritative where the references still
              support it.
            </p>
          </div>
        )
      ) : (
        content && (
          <div className="doc-body">
            <article className="markdown">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{content.markdown}</ReactMarkdown>
            </article>
            <aside className="tokens">
              <CmfSwatches tokens={content.tokens} />
              <h3>tokens.json</h3>
              <pre className="json">{JSON.stringify(content.tokens, null, 2)}</pre>
            </aside>
          </div>
        )
      )}
    </div>
  );
}
