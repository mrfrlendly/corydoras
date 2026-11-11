import { DragEvent, FormEvent, KeyboardEvent as ReactKeyboardEvent, useCallback, useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import {
  BookOpen,
  CircleAlert,
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

/** The state of the analysis in one word, for the card label. */
function statusWord(s: Source): string {
  switch (s.analysis_status) {
    case 'done':
      return 'Read';
    case 'error':
      return 'Failed';
    case 'analyzing':
      return 'Reading';
    default:
      return 'Waiting';
  }
}

/** The state of the analysis in words; nothing when the model has read the source. */
function SourceStatus({ source }: { source: Source }) {
  switch (source.analysis_status) {
    case 'done':
      return null;
    case 'error':
      return (
        <span className="cap-status cap-error" title={source.analysis_error || 'The model could not read this source.'}>
          <CircleAlert size={11} aria-hidden="true" /> Failed
        </span>
      );
    case 'analyzing':
      return (
        <span className="cap-status cap-busy">
          <Loader size={11} className="spin" aria-hidden="true" /> Reading
        </span>
      );
    default:
      return <span className="cap-status">Waiting</span>;
  }
}

/** A text field has the keys; the view must not act on them. */
function inField(t: EventTarget | null) {
  return t instanceof HTMLElement && !!t.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])');
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
    <figure
      className={`src-card ${selected ? 'selected' : ''}`}
      data-card={source.id}
      tabIndex={0}
      role="button"
      aria-pressed={selected}
      aria-label={`${source.title}. ${statusWord(source)}. ${mediaLabel(source.media)}. Weight ${source.weight} of 5.`}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
          e.preventDefault();
          onClick();
        }
      }}
    >
      <Preview source={source} />
      <figcaption>
        <div className="src-title" title={source.title}>
          {source.title}
        </div>
        <div className="src-meta">
          <SourceStatus source={source} />
          <MediaIcon media={source.media} size={12} />
          <span className="src-site">{source.site || source.original_name || mediaLabel(source.media)}</span>
          <span className="src-words">{fmtWords(source.word_count)}</span>
          <span className="weight" role="img" aria-label={`Weight ${source.weight} of 5`} title={`Weight ${source.weight} of 5`}>
            {[1, 2, 3, 4, 5].map((n) => (
              <i key={n} className={n <= source.weight ? 'on' : ''} />
            ))}
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
  const closeRef = useRef<HTMLButtonElement>(null);
  const removeRef = useRef<HTMLButtonElement>(null);
  const refocusRemove = useRef(false);
  /** Set when "Remove from language" starts: a removed source has nothing to save. */
  const removed = useRef(false);
  /** The last title sent with saveTitle or save, so that the unmount does not send it again. */
  const sentTitle = useRef(source.title);
  /** The last note sent with save (trimmed, as the library keeps it). Until the list reloads,
   *  `source.note` is the old note: the unmount must not call the new one unsaved, and a title
   *  or weight change must not send the old one back (that makes the model read the source again). */
  const sentNote = useRef<string | null>(null);
  const latest = useRef({ source, title, note, weight, onChanged });
  latest.current = { source, title, note, weight, onChanged };

  useEffect(() => {
    api
      .sourceDetail(source.id)
      .then(setDetail)
      .catch((e) => toast.error(errText(e)));
  }, [source.id, source.analysis_status, tick, toast]);

  // The inspector closes when another card opens. A changed title is saved then; a changed note is not,
  // because a new note makes the model read the source again, so a message offers to save it.
  useEffect(
    () => () => {
      if (removed.current) return;
      const { source: s, title: t0, note: n, weight: w, onChanged: changed } = latest.current;
      const t = t0.trim() || s.title;
      const savedNote = sentNote.current ?? s.note;
      const fail = (e: unknown) => toast.error(errText(e));
      if (t !== s.title && t !== sentTitle.current) api.updateSource(s.id, t, savedNote, w).then(changed).catch(fail);
      if (n.trim() !== savedNote) {
        toast.info(`The note for “${t}” is not saved.`, {
          label: 'Save',
          run: () => {
            api.updateSource(s.id, t, n, w).then(changed).catch(fail);
          },
        });
      }
    },
    [toast],
  );

  useEffect(() => {
    if (!confirmRemove && refocusRemove.current) {
      refocusRemove.current = false;
      removeRef.current?.focus();
    }
  }, [confirmRemove]);

  /** Saves the note (it makes the model read the source again), with the title and the weight. */
  const save = async () => {
    const t = title.trim() || source.title;
    const before = { note: sentNote.current, title: sentTitle.current };
    sentNote.current = note.trim();
    sentTitle.current = t;
    try {
      await api.updateSource(source.id, t, note, weight);
      onChanged();
    } catch (e) {
      sentNote.current = before.note;
      sentTitle.current = before.title;
      toast.error(errText(e));
    }
  };

  /** Saves the title and the weight; the saved note stays, so the source is not read again. */
  const saveQuiet = async (w: number) => {
    try {
      await api.updateSource(source.id, title.trim() || source.title, sentNote.current ?? source.note, w);
      onChanged();
      return true;
    } catch (e) {
      toast.error(errText(e));
      return false;
    }
  };

  const saveTitle = () => {
    const t = title.trim();
    if (!t) setTitle(source.title);
    else if (t !== source.title && t !== sentTitle.current) {
      sentTitle.current = t;
      saveQuiet(weight).then((ok) => {
        if (!ok) sentTitle.current = source.title;
      });
    }
  };

  const cancelRemove = () => {
    refocusRemove.current = true;
    setConfirmRemove(false);
  };

  // Escape in a field leaves the field; elsewhere the window handler of the view closes the inspector.
  const onKeyDown = (e: ReactKeyboardEvent) => {
    if (e.key !== 'Escape' || !inField(e.target)) return;
    e.preventDefault();
    (e.target as HTMLElement).blur();
    closeRef.current?.focus();
  };

  const openIt = () => api.openSource(source.id).catch((e) => toast.error(errText(e)));
  const isPdf = source.media === 'pdf' && !!source.file_rel;

  return (
    <aside className="inspector src-inspector" aria-labelledby="src-insp-title" onKeyDown={onKeyDown}>
      <div className="insp-head">
        <div>
          <div className="eyebrow">
            Source {shortId(source.id)} · {mediaLabel(source.media)}
          </div>
          <div className="insp-name" id="src-insp-title" title={source.title}>
            {source.title}
          </div>
        </div>
        <button ref={closeRef} className="icon-btn" onClick={onClose} title="Close" aria-label="Close source">
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
            <input value={title} onChange={(e) => setTitle(e.target.value)} onBlur={saveTitle} onKeyDown={(e) => e.key === 'Enter' && e.currentTarget.blur()} />
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
            <span id="src-weight-label">Weight</span>
            <div className="weights" role="group" aria-labelledby="src-weight-label">
              {[1, 2, 3, 4, 5].map((w) => (
                <button
                  key={w}
                  className={`weight-btn ${w < weight ? 'on' : ''} ${w === weight ? 'current' : ''}`}
                  aria-pressed={w === weight}
                  aria-label={`Weight ${w} of 5`}
                  onClick={() => {
                    setWeight(w);
                    saveQuiet(w);
                  }}
                >
                  {w}
                </button>
              ))}
            </div>
          </div>
          {note.trim() !== source.note && (
            <div className="row">
              <button className="btn primary small" onClick={save}>
                Save note
              </button>
              <span className="muted small">A new note makes the model read the source again.</span>
            </div>
          )}
        </section>

        <section className="insp-section">
          <h3>
            Notes from the model <SourceStatus source={source} />
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
            <div
              className="row"
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  e.preventDefault();
                  cancelRemove();
                }
              }}
            >
              <button
                className="btn danger small"
                onClick={() => {
                  if (removed.current) return;
                  // Set before the call: an arrow key during the removal must not save this source on close.
                  removed.current = true;
                  api
                    .removeSource(source.id)
                    .then(onRemoved)
                    .catch((e) => {
                      removed.current = false;
                      toast.error(errText(e));
                    });
                }}
              >
                <Trash2 size={13} /> Remove from language
              </button>
              <button className="btn small" autoFocus onClick={cancelRemove}>
                Cancel
              </button>
            </div>
          ) : (
            <button ref={removeRef} className="btn small ghost-danger" onClick={() => setConfirmRemove(true)}>
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
  const rootRef = useRef<HTMLDivElement>(null);

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
  /** The selection now, for callbacks that end after other keys or clicks changed it. */
  const selectedRef = useRef(selected);
  selectedRef.current = selected;

  /** Moves the keyboard focus to a card, after the layout has followed the selection. */
  const focusCard = (id: string) =>
    requestAnimationFrame(() => {
      const el = rootRef.current?.querySelector<HTMLElement>(`[data-card="${CSS.escape(id)}"]`);
      if (!el) return;
      el.focus({ preventScroll: true });
      el.scrollIntoView({ block: 'nearest' });
    });

  const close = () => {
    const id = selected;
    setSelected(null);
    if (id) focusCard(id);
  };

  // ← and → open the previous and the next source; Escape closes the inspector. Only while the focus is in
  // this view (or nowhere), and never in a text field or with ⌘, ⌃ or ⌥.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || inField(e.target)) return;
      const t = e.target;
      if (t instanceof Node && t !== document.body && !rootRef.current?.contains(t)) return;
      if (e.key === 'Escape') {
        if (!selected) return;
        e.preventDefault();
        close();
        return;
      }
      if ((e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') || sources.length === 0) return;
      const step = e.key === 'ArrowRight' ? 1 : -1;
      const i = sources.findIndex((x) => x.id === selected);
      const next = i < 0 ? (step > 0 ? 0 : sources.length - 1) : Math.min(sources.length - 1, Math.max(0, i + step));
      const id = sources[next].id;
      if (id === selected) return;
      e.preventDefault();
      setSelected(id);
      focusCard(id);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  return (
    <div
      ref={rootRef}
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
            <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="Paste a link to a tutorial, article, video or PDF" aria-label="Link" spellCheck={false} autoCorrect="off" autoCapitalize="off" />
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
          onClose={close}
          onChanged={changed}
          onRemoved={() => {
            const removedId = selectedSource.id;
            changed();
            // The removal takes a moment: when another source was opened meanwhile, it stays open.
            if (selectedRef.current !== removedId) return;
            // The focus goes to the card after the removed one (or before it, at the end).
            const i = sources.findIndex((x) => x.id === removedId);
            const next = i < 0 ? undefined : (sources[i + 1] ?? sources[i - 1]);
            setSelected((cur) => (cur === removedId ? null : cur));
            if (next) focusCard(next.id);
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
