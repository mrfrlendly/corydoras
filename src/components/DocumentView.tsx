import { useEffect, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { api, errText, fmtDate, Language, VersionContent, VersionInfo } from '../api';
import { Check, Copy, Pencil } from 'lucide-react';
import { useToast } from './Toast';

interface Props {
  language: Language;
  version: number | null;
  tick: number;
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

export default function DocumentView({ language, version, tick, onVersion, onChanged }: Props) {
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
              Your edit becomes a new version after the same validation that the model output gets. Later automatic versions treat your edit
              as authoritative where the references still support it.
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
