import { DragEvent, FormEvent, useCallback, useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import {
  BookOpen,
  CirclePlay,
  ExternalLink,
  FilePlus,
  FileText,
  FileType,
  Globe,
  Link2,
  Loader,
  Plus,
  RefreshCw,
  Trash2,
  X,
} from 'lucide-react';
import {
  api,
  DOC_EXT_RE,
  DOC_EXTENSIONS,
  errText,
  fileToBase64,
  fileUrl,
  fmtDate,
  fmtWords,
  IMAGE_URL_RE,
  Language,
  mediaLabel,
  shortId,
  Source,
  SourceDetail,
} from '../api';
import { useToast } from './Toast';

/** The glyph for a kind of source. */
export function MediaIcon({ media, size = 16 }: { media: string; size?: number }) {
  switch (media) {
    case 'video':
      return <CirclePlay size={size} strokeWidth={1.6} />;
    case 'page':
      return <Globe size={size} strokeWidth={1.6} />;
    case 'pdf':
      return <FileType size={size} strokeWidth={1.6} />;
    default:
      return <FileText size={size} strokeWidth={1.6} />;
  }
}

function statusLabel(s: Source): string {
  switch (s.analysis_status) {
    case 'done':
      return 'Read';
    case 'error':
      return `Failed: ${s.analysis_error ?? ''}`;
    case 'analyzing':
      return 'Reading now';
    default:
      return 'In the queue';
  }
}

/** Preview of a source: its image, or a tile with a glyph and the first words. */
function Preview({ source, large }: { source: Source; large?: boolean }) {
  const [broken, setBroken] = useState(false);
  if (source.preview_rel && !broken) {
    return (
      <div className={`src-preview ${large ? 'large' : ''}`}>
        <img src={fileUrl(source.preview_rel)} alt="" draggable={false} onError={() => setBroken(true)} />
        {source.media === 'video' && (
          <span className="src-play">
            <CirclePlay size={large ? 44 : 30} strokeWidth={1.4} />
          </span>
        )}
      </div>
    );
  }
  return (
    <div className={`src-preview tile media-${source.media} ${large ? 'large' : ''}`}>
      <MediaIcon media={source.media} size={large ? 40 : 28} />
      <span className="src-tile-label">{mediaLabel(source.media)}</span>
      {source.description && <span className="src-tile-text">{source.description}</span>}
    </div>
  );
}

function SourceCard({ source, selected, onClick }: { source: Source; selected: boolean; onClick: () => void }) {
  return (
    <figure className={`src-card ${selected ? 'selected' : ''}`} onClick={onClick}>
      <Preview source={source} />
      <figcaption>
        <div className="src-title" title={source.title}>
          {source.title}
        </div>
        <div className="src-meta">
          <span className={`dot status-${source.analysis_status}`} title={statusLabel(source)} />
          <MediaIcon media={source.media} size={12} />
          <span className="src-site">{source.site || source.original_name || mediaLabel(source.media)}</span>
          <span className="src-words">{fmtWords(source.word_count)}</span>
          <span className="weight" title={`Weight ${source.weight}/5`}>
            {'●'.repeat(source.weight)}
            <span className="weight-off">{'●'.repeat(5 - source.weight)}</span>
          </span>
        </div>
      </figcaption>
    </figure>
  );
}

function List({ title, items }: { title: string; items: string[] }) {
  if (items.length === 0) return null;
  return (
    <div className="src-list">
      <h4>{title}</h4>
      <ul>
        {items.map((x, i) => (
          <li key={i}>{x}</li>
        ))}
      </ul>
    </div>
  );
}

const arr = (v: unknown): Record<string, unknown>[] => (Array.isArray(v) ? (v as Record<string, unknown>[]) : []);
const strs = (v: unknown): string[] => (Array.isArray(v) ? (v as unknown[]).map(String).filter(Boolean) : []);
const str = (v: unknown): string => (typeof v === 'string' ? v : v == null ? '' : String(v));

function Notes({ analysis, model }: { analysis: Record<string, unknown>; model: string | null }) {
  const [raw, setRaw] = useState(false);
  const principles = arr(analysis.principles);
  const techniques = arr(analysis.techniques);
  const values = arr(analysis.values);
  const definitions = arr(analysis.definitions);
  const topics = strs(analysis.topics);
  return (
    <div className="analysis src-notes">
      {str(analysis.summary) && <p>{str(analysis.summary)}</p>}
      {topics.length > 0 && <div className="kw">{topics.join(' · ')}</div>}
      <List title="Principles" items={principles.map((p) => str(p.text) + (str(p.applies_to) ? ` — ${str(p.applies_to)}` : ''))} />
      <List title="Techniques" items={techniques.map((t) => `${str(t.name)}: ${str(t.how)}`)} />
      {values.length > 0 && (
        <div className="src-list">
          <h4>Values</h4>
          <table className="src-values">
            <tbody>
              {values.map((v, i) => (
                <tr key={i}>
                  <td>{str(v.name)}</td>
                  <td className="mono">
                    {str(v.value)} {str(v.unit)}
                  </td>
                  <td className="muted">{[str(v.of), str(v.context)].filter(Boolean).join(' · ')}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <List title="Do" items={strs(analysis.do)} />
      <List title="Do not" items={strs(analysis.dont)} />
      <List title="Definitions" items={definitions.map((d) => `${str(d.term)}: ${str(d.meaning)}`)} />
      {str(analysis.relevance) && (
        <div className="src-list">
          <h4>Relevance</h4>
          <p>{str(analysis.relevance)}</p>
        </div>
      )}
      {str(analysis.limits) && (
        <div className="src-list">
          <h4>Limits</h4>
          <p className="muted">{str(analysis.limits)}</p>
        </div>
      )}
      <button className="link-btn" onClick={() => setRaw(!raw)}>
        {raw ? 'Hide' : 'Show'} full notes{model ? ` (${model})` : ''}
      </button>
      {raw && <pre className="json">{JSON.stringify(analysis, null, 2)}</pre>}
    </div>
  );
}

function SourceInspector({
  source,
  tick,
  onClose,
  onChanged,
  onRemoved,
}: {
  source: Source;
  tick: number;
  onClose: () => void;
  onChanged: () => void;
  onRemoved: () => void;
}) {
  const toast = useToast();
  const [detail, setDetail] = useState<SourceDetail | null>(null);
  const [title, setTitle] = useState(source.title);
  const [note, setNote] = useState(source.note);
  const [weight, setWeight] = useState(source.weight);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [showText, setShowText] = useState(false);

  useEffect(() => {
    api
      .sourceDetail(source.id)
      .then(setDetail)
      .catch((e) => toast.error(errText(e)));
  }, [source.id, source.analysis_status, tick, toast]);

  const save = async (w?: number) => {
    try {
      await api.updateSource(source.id, title.trim() || source.title, note, w ?? weight);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const openIt = () => api.openSource(source.id).catch((e) => toast.error(errText(e)));
  const isPdf = source.media === 'pdf' && !!source.file_rel;

  return (
    <aside className="inspector src-inspector">
      <div className="insp-head">
        <div>
          <div className="eyebrow">
            Source {shortId(source.id)} · {mediaLabel(source.media)}
          </div>
          <div className="insp-name" title={source.title}>
            {source.title}
          </div>
        </div>
        <button className="icon-btn" onClick={onClose} title="Close">
          <X size={15} />
        </button>
      </div>

      <div className="insp-scroll">
        {isPdf ? (
          <iframe className="pdf-frame" src={fileUrl(source.file_rel!)} title={source.title} />
        ) : (
          <Preview source={source} large />
        )}
        <div className="row src-open-row">
          <button className="btn small primary" onClick={openIt}>
            <ExternalLink size={13} />
            {source.url ? 'Open link' : 'Open document'}
          </button>
          <span className="muted small wrap">{source.url ?? source.original_name}</span>
        </div>
        <div className="frame-hint">
          {fmtWords(source.word_count)} · added {fmtDate(source.added_at)}
          {source.media === 'video' && ' · The analysis reads only the title and the description of a video.'}
        </div>

        <section className="insp-section">
          <h3>Annotation</h3>
          <label className="field">
            <span>Title</span>
            <input value={title} onChange={(e) => setTitle(e.target.value)} />
          </label>
          <label className="field">
            <span>Note — what must the language take from this source?</span>
            <textarea
              rows={3}
              value={note}
              onChange={(e) => setNote(e.target.value)}
              placeholder="For example: use the fillet and draft rules; ignore the colour advice."
            />
          </label>
          <div className="field">
            <span>Weight</span>
            <div className="weights">
              {[1, 2, 3, 4, 5].map((w) => (
                <button
                  key={w}
                  className={`weight-btn ${w <= weight ? 'on' : ''}`}
                  onClick={() => {
                    setWeight(w);
                    save(w);
                  }}
                >
                  {w}
                </button>
              ))}
            </div>
          </div>
          {(title !== source.title || note !== source.note) && (
            <div className="row">
              <button className="btn primary small" onClick={() => save()}>
                Save annotation
              </button>
              {note !== source.note && <span className="muted small">A new note makes the model read the source again.</span>}
            </div>
          )}
        </section>

        <section className="insp-section">
          <h3>
            Notes from the model <span className={`dot status-${source.analysis_status}`} />
          </h3>
          {source.analysis_status === 'error' && <div className="error-box">{source.analysis_error}</div>}
          {source.analysis_status === 'pending' && <div className="muted small">In the queue.</div>}
          {source.analysis_status === 'analyzing' && (
            <div className="muted small row">
              <Loader size={13} className="spin" /> The model reads this source now.
            </div>
          )}
          {detail?.analysis && <Notes analysis={detail.analysis} model={detail.analysis_model} />}
          <div className="row">
            <button
              className="btn small"
              onClick={() =>
                api
                  .reanalyzeSource(source.id)
                  .then(onChanged)
                  .catch((e) => toast.error(errText(e)))
              }
            >
              <RefreshCw size={13} /> Read again
            </button>
          </div>
        </section>

        <section className="insp-section">
          <h3>Extracted text</h3>
          {detail && detail.text_preview ? (
            <>
              <div className={`text-preview ${showText ? 'open' : ''}`}>{detail.text_preview}</div>
              <button className="link-btn" onClick={() => setShowText(!showText)}>
                {showText ? 'Show less' : 'Show more'}
              </button>
            </>
          ) : (
            <div className="muted small">No text.</div>
          )}
        </section>

        <section className="insp-section">
          {confirmRemove ? (
            <div className="row">
              <button
                className="btn danger small"
                onClick={() =>
                  api
                    .removeSource(source.id)
                    .then(onRemoved)
                    .catch((e) => toast.error(errText(e)))
                }
              >
                <Trash2 size={13} /> Remove from language
              </button>
              <button className="btn small" onClick={() => setConfirmRemove(false)}>
                Cancel
              </button>
            </div>
          ) : (
            <button className="btn small ghost-danger" onClick={() => setConfirmRemove(true)}>
              <Trash2 size={13} /> Remove…
            </button>
          )}
        </section>
      </div>
    </aside>
  );
}

function firstUri(list: string): string | null {
  return (
    list
      .split(/\r?\n/)
      .map((l) => l.trim())
      .find((l) => l && !l.startsWith('#')) ?? null
  );
}

export default function SourcesView({ language, tick, onChanged }: { language: Language; tick: number; onChanged: () => void }) {
  const toast = useToast();
  const [sources, setSources] = useState<Source[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [url, setUrl] = useState('');
  const [busy, setBusy] = useState(0);
  const [dragging, setDragging] = useState(false);
  const dragDepth = useRef(0);

  const load = useCallback(() => {
    api
      .listSources(language.id)
      .then(setSources)
      .catch((e) => toast.error(errText(e)));
  }, [language.id, toast]);
  useEffect(load, [load, tick]);

  const changed = useCallback(() => {
    load();
    onChanged();
  }, [load, onChanged]);

  const work = async (fn: () => Promise<void>) => {
    setBusy((b) => b + 1);
    try {
      await fn();
    } finally {
      setBusy((b) => b - 1);
    }
  };

  const addLink = (link: string) =>
    work(async () => {
      try {
        const s = await api.addLink(language.id, link);
        toast.info(`Added “${s.title}”. The model reads it next.`);
        setSelected(s.id);
        changed();
      } catch (e) {
        toast.error(errText(e));
      }
    });

  const submit = (e: FormEvent) => {
    e.preventDefault();
    let link = url.trim();
    if (!link) return;
    if (!/^https?:\/\//i.test(link)) link = 'https://' + link;
    setUrl('');
    addLink(link);
  };

  const addFiles = (files: File[]) =>
    work(async () => {
      let added = 0;
      for (const f of files) {
        if (!DOC_EXT_RE.test(f.name)) {
          toast.error(`“${f.name}” is not a document that Corydoras reads (${DOC_EXTENSIONS.join(', ')}).`);
          continue;
        }
        try {
          await api.importSourceBytes(language.id, f.name, await fileToBase64(f));
          added++;
        } catch (e) {
          toast.error(errText(e));
        }
      }
      if (added) toast.info(`${added} document${added === 1 ? '' : 's'} added. The model reads them next.`);
      changed();
    });

  const addFromDialog = async () => {
    const picked = await open({ multiple: true, title: 'Add documents', filters: [{ name: 'Documents', extensions: DOC_EXTENSIONS }] });
    const paths = Array.isArray(picked) ? picked : typeof picked === 'string' ? [picked] : [];
    if (paths.length === 0) return;
    work(async () => {
      try {
        const r = await api.importSourceFiles(language.id, paths);
        r.errors.forEach((e) => toast.error(e));
        if (r.added.length) toast.info(`${r.added.length} document${r.added.length === 1 ? '' : 's'} added. The model reads them next.`);
        changed();
      } catch (e) {
        toast.error(errText(e));
      }
    });
  };

  // Paste a link or a document with Cmd+V.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA')) return;
      const files = Array.from(e.clipboardData?.files ?? []).filter((f) => DOC_EXT_RE.test(f.name));
      if (files.length > 0) {
        e.preventDefault();
        addFiles(files);
        return;
      }
      const text = e.clipboardData?.getData('text/plain')?.trim() ?? '';
      if (/^https?:\/\/\S+$/.test(text)) {
        e.preventDefault();
        addLink(text);
      }
    };
    document.addEventListener('paste', onPaste);
    return () => document.removeEventListener('paste', onPaste);
  });

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    dragDepth.current = 0;
    setDragging(false);
    const files = Array.from(e.dataTransfer.files);
    if (files.length > 0) {
      addFiles(files);
      return;
    }
    const uri = firstUri(e.dataTransfer.getData('text/uri-list')) ?? e.dataTransfer.getData('text/plain').trim();
    if (uri && /^https?:\/\//.test(uri)) {
      if (IMAGE_URL_RE.test(uri)) {
        toast.error('That link is an image. Add images on the Board.');
        return;
      }
      addLink(uri);
      return;
    }
    toast.error('Drop a document, or a link from a browser.');
  };

  const selectedSource = sources.find((s) => s.id === selected) ?? null;

  return (
    <div
      className={`board-wrap ${selectedSource ? 'with-inspector wide' : ''}`}
      onDragEnter={(e) => {
        e.preventDefault();
        dragDepth.current++;
        setDragging(true);
      }}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = 'copy';
      }}
      onDragLeave={() => {
        dragDepth.current = Math.max(0, dragDepth.current - 1);
        if (dragDepth.current === 0) setDragging(false);
      }}
      onDrop={onDrop}
    >
      <div className="board-main">
        <div className="toolbar">
          <form className="link-form" onSubmit={submit}>
            <Link2 size={14} className="link-form-icon" />
            <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="Paste a link to a tutorial, article, video or PDF" />
            <button className="btn small primary" type="submit" disabled={!url.trim()}>
              <Plus size={13} /> Add link
            </button>
          </form>
          <button className="btn small" onClick={addFromDialog}>
            <FilePlus size={13} /> Add documents…
          </button>
          <div className="spacer" />
          {busy > 0 && (
            <span className="chip chip-busy">
              <Loader size={12} className="spin" /> Reading…
            </span>
          )}
        </div>

        {sources.length === 0 ? (
          <div className="board-empty">
            <div className="drop-target">
              <BookOpen size={34} strokeWidth={1.3} className="drop-glyph" />
              <div className="drop-big">Add tutorials and documents</div>
              <p>
                Paste a link above, or drop PDF, Word, text or Markdown files here. The model reads each source and extracts the principles,
                techniques and values. The synthesis uses these notes with your images, and rules from a source cite its ID.
              </p>
              <button className="btn primary" onClick={addFromDialog}>
                <FilePlus size={14} /> Add documents…
              </button>
            </div>
          </div>
        ) : (
          <div className="src-grid">
            {sources.map((s) => (
              <SourceCard key={s.id} source={s} selected={s.id === selected} onClick={() => setSelected(s.id === selected ? null : s.id)} />
            ))}
          </div>
        )}
      </div>

      {selectedSource && (
        <SourceInspector
          key={selectedSource.id}
          source={selectedSource}
          tick={tick}
          onClose={() => setSelected(null)}
          onChanged={changed}
          onRemoved={() => {
            setSelected(null);
            changed();
          }}
        />
      )}

      {dragging && (
        <div className="drop-overlay">
          <div>Drop to add a source to {language.name}</div>
        </div>
      )}
    </div>
  );
}
