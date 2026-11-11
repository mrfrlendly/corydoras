import { KeyboardEvent as ReactKeyboardEvent, PointerEvent, useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { api, errText, fileUrl, fmtDate, Item, ItemDetail, RegionInput, shortId } from '../api';
import { RefreshCw, Trash2, X } from 'lucide-react';
import { useToast } from './Toast';

interface Props {
  item: Item;
  tick: number;
  /** The keyboard opened the inspector: focus its close button. */
  focusOnOpen?: boolean;
  onClose: () => void;
  onChanged: () => void;
  onRemoved: () => void;
}

const parseTags = (t: string) =>
  t
    .split(',')
    .map((x) => x.trim())
    .filter(Boolean);

/** Keys in these places belong to the field. */
const isField = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || !!t.closest('input, textarea, select'));

type Draft = RegionInput & { key: string };

let keySeq = 0;
const newKey = () => `k${++keySeq}`;

function toDrafts(d: ItemDetail | null): Draft[] {
  return (d?.regions ?? []).map((r) => ({ ...r, id: r.id, key: newKey() }));
}

const toInputs = (rs: Draft[]): RegionInput[] => rs.map(({ id, x, y, w, h, note, tag }) => ({ id, x, y, w, h, note, tag }));

function AnalysisBlock({ analysis, model }: { analysis: Record<string, unknown>; model: string | null }) {
  const [raw, setRaw] = useState(false);
  const summary = typeof analysis.summary === 'string' ? analysis.summary : '';
  const keywords = Array.isArray(analysis.keywords) ? (analysis.keywords as unknown[]).map(String) : [];
  const colours = Array.isArray(analysis.colours) ? (analysis.colours as Record<string, unknown>[]) : [];
  return (
    <div className="analysis">
      {summary && <p>{summary}</p>}
      {colours.length > 0 && (
        <div className="swatches">
          {colours.map((c, i) => {
            const hex = typeof c.hex === 'string' ? c.hex : '#888888';
            return (
              <span key={i} className="swatch" title={`${String(c.role ?? '')} ${String(c.name ?? '')} ${hex}`}>
                <span className="swatch-color" style={{ background: hex }} />
                {hex}
              </span>
            );
          })}
        </div>
      )}
      {keywords.length > 0 && <div className="kw">{keywords.join(' · ')}</div>}
      <button className="link-btn" onClick={() => setRaw(!raw)}>
        {raw ? 'Hide' : 'Show'} full analysis{model ? ` (${model})` : ''}
      </button>
      {raw && <pre className="json">{JSON.stringify(analysis, null, 2)}</pre>}
    </div>
  );
}

export default function ItemInspector({ item, tick, focusOnOpen = false, onClose, onChanged, onRemoved }: Props) {
  const toast = useToast();
  const [detail, setDetail] = useState<ItemDetail | null>(null);
  const [tags, setTags] = useState(item.tags.join(', '));
  const [note, setNote] = useState(item.note);
  const [weight, setWeight] = useState(item.weight);
  const [anti, setAnti] = useState(item.anti);
  const [regions, setRegions] = useState<Draft[]>([]);
  const [regionsDirty, setRegionsDirty] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [drawing, setDrawing] = useState<{ x0: number; y0: number; x: number; y: number } | null>(null);
  const asideRef = useRef<HTMLElement>(null);
  const frameRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const removeRef = useRef<HTMLButtonElement>(null);
  const refocusRemove = useRef(false);
  /** Set when "Remove from language" starts: a removed item has nothing to save. */
  const removing = useRef(false);

  // Tags and note save when the field loses focus, and when the inspector closes
  // or shows another item (it is keyed by item), so typed text is never lost.
  const dirty = useRef(false);
  const latest = useRef({ tags, note, weight, anti, onChanged });
  latest.current = { tags, note, weight, anti, onChanged };
  // Marks do not save on close: a save analyzes the item again (it costs tokens), so a message asks.
  // `sent` is the list that saveRegions is sending, so that a close during that save does not ask.
  const marks = useRef<{ regions: Draft[]; dirty: boolean; sent: Draft[] | null }>({ regions, dirty: regionsDirty, sent: null });
  marks.current.regions = regions;
  marks.current.dirty = regionsDirty;

  useEffect(() => {
    api
      .itemDetail(item.id)
      .then((d) => {
        setDetail(d);
        if (!regionsDirty) setRegions(toDrafts(d));
      })
      .catch((e) => toast.error(errText(e)));
    // regionsDirty is read on purpose without a dependency: a reload must not drop unsaved marks.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item.id, tick, item.analysis_status, toast]);

  useEffect(() => {
    if (focusOnOpen) closeRef.current?.focus();
    // Only when this item opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(
    () => () => {
      const l = latest.current;
      const m = marks.current;
      if (!removing.current && m.dirty && m.regions !== m.sent) {
        toast.info(`The changes to the marks on “${item.original_name}” are not saved.`, {
          label: 'Save',
          run: () => {
            api
              .setRegions(item.id, toInputs(m.regions))
              .then(() => {
                toast.info('Marks saved. The item gets analyzed again with the crops.');
                l.onChanged();
              })
              .catch((e) => toast.error(errText(e)));
          },
        });
      }
      if (!dirty.current) return;
      dirty.current = false;
      api
        .updateItem(item.id, parseTags(l.tags), l.note, l.weight, l.anti)
        .then(() => l.onChanged())
        .catch((e) => toast.error(errText(e)));
    },
    // The save on close reads the latest values from the refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  const saveAnnotation = async (over?: Partial<{ weight: number; anti: boolean }>) => {
    dirty.current = false;
    const l = latest.current;
    try {
      await api.updateItem(item.id, parseTags(l.tags), l.note, over?.weight ?? l.weight, over?.anti ?? l.anti);
      onChanged();
    } catch (e) {
      dirty.current = true;
      toast.error(errText(e));
    }
  };
  const saveIfDirty = () => {
    if (dirty.current) saveAnnotation();
  };

  const setWeightNow = (w: number) => {
    if (w === latest.current.weight) return;
    setWeight(w);
    latest.current.weight = w;
    saveAnnotation({ weight: w });
  };

  // 1 to 5 set the weight, outside text fields. Only keys pressed on this board count (or with the focus
  // nowhere), and not on the card of another item.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || isField(e.target)) return;
      if (!/^[1-5]$/.test(e.key)) return;
      const t = e.target;
      if (t instanceof Node && t !== document.body) {
        if (!asideRef.current?.closest('.board-wrap')?.contains(t)) return;
        const card = t instanceof Element ? t.closest<HTMLElement>('[data-card]') : null;
        if (card && card.dataset.card !== item.id) return;
      }
      e.preventDefault();
      setWeightNow(Number(e.key));
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // Back to "Remove…" after its confirmation is cancelled.
  useEffect(() => {
    if (!confirmRemove && refocusRemove.current) {
      refocusRemove.current = false;
      removeRef.current?.focus();
    }
  }, [confirmRemove]);

  const cancelRemove = () => {
    refocusRemove.current = true;
    setConfirmRemove(false);
  };

  // Escape: cancels the remove confirmation; in a field it leaves the field; elsewhere it closes.
  const onKeyDown = (e: ReactKeyboardEvent<HTMLElement>) => {
    if (e.key !== 'Escape' || e.defaultPrevented) return;
    e.preventDefault();
    if (confirmRemove) {
      cancelRemove();
    } else if (isField(e.target)) {
      (e.target as HTMLElement).blur();
      closeRef.current?.focus();
    } else {
      onClose();
    }
  };

  const reanalyze = () =>
    api
      .reanalyzeItem(item.id)
      .then(onChanged)
      .catch((e) => toast.error(errText(e)));

  const saveRegions = async () => {
    marks.current.sent = regions;
    try {
      await api.setRegions(item.id, toInputs(regions));
      setRegionsDirty(false);
      toast.info('Marks saved. The item gets analyzed again with the crops.');
      onChanged();
    } catch (e) {
      marks.current.sent = null;
      toast.error(errText(e));
    }
  };

  const pos = (e: PointerEvent) => {
    const r = frameRef.current!.getBoundingClientRect();
    return {
      x: Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)),
      y: Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)),
    };
  };

  // The ref holds the live drag, so a fast release never reads a stale render.
  const dragRef = useRef<{ x0: number; y0: number; x: number; y: number } | null>(null);
  const onDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const p = pos(e);
    dragRef.current = { x0: p.x, y0: p.y, x: p.x, y: p.y };
    setDrawing(dragRef.current);
  };
  const onMove = (e: PointerEvent) => {
    if (!dragRef.current) return;
    const p = pos(e);
    dragRef.current = { ...dragRef.current, x: p.x, y: p.y };
    setDrawing(dragRef.current);
  };
  const onUp = (e: PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    const p = pos(e);
    d.x = p.x;
    d.y = p.y;
    dragRef.current = null;
    const x = Math.min(d.x0, d.x);
    const y = Math.min(d.y0, d.y);
    const w = Math.abs(d.x - d.x0);
    const h = Math.abs(d.y - d.y0);
    setDrawing(null);
    if (w < 0.02 || h < 0.02) return;
    setRegions((rs) => [...rs, { id: null, x, y, w, h, note: '', tag: '', key: newKey() }]);
    setRegionsDirty(true);
  };

  const live = drawing
    ? {
        x: Math.min(drawing.x0, drawing.x),
        y: Math.min(drawing.y0, drawing.y),
        w: Math.abs(drawing.x - drawing.x0),
        h: Math.abs(drawing.y - drawing.y0),
      }
    : null;

  const src = item.source;
  const isWeb = !!src && /^https?:\/\//.test(src);
  const statusLine =
    item.analysis_status === 'done'
      ? `Analyzed${detail?.analysis_model ? ` by ${detail.analysis_model}` : ''}.`
      : item.analysis_status === 'analyzing'
        ? 'The model reads this image now.'
        : 'Waiting for analysis.';

  return (
    <aside ref={asideRef} className="inspector" aria-labelledby="insp-title" onKeyDown={onKeyDown}>
      <div className="insp-head">
        <div>
          <div className="eyebrow">Item {shortId(item.id)}</div>
          <div className="insp-name" id="insp-title" title={item.original_name}>
            {item.original_name}
          </div>
        </div>
        <button ref={closeRef} className="icon-btn" onClick={onClose} title="Close" aria-label="Close item">
          <X size={15} />
        </button>
      </div>

      <div className="insp-scroll">
        <div
          className="frame"
          ref={frameRef}
          // A tall image would push everything below the fold: cap its height by narrowing the
          // frame (the marks use the frame's box, so the image must fill it exactly).
          style={{ width: `min(100%, calc(40vh * ${Math.max(1, item.width)} / ${Math.max(1, item.height)}))` }}
        >
          <img src={fileUrl(item.file_rel)} alt={item.original_name} draggable={false} />
          <div className="frame-overlay" onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp}>
            {regions.map((r, i) => (
              <div
                key={r.key}
                className="mark"
                style={{ left: `${r.x * 100}%`, top: `${r.y * 100}%`, width: `${r.w * 100}%`, height: `${r.h * 100}%` }}
              >
                <span className="mark-num">r{i + 1}</span>
              </div>
            ))}
            {live && (
              <div
                className="mark live"
                style={{ left: `${live.x * 100}%`, top: `${live.y * 100}%`, width: `${live.w * 100}%`, height: `${live.h * 100}%` }}
              />
            )}
          </div>
        </div>
        <div className="frame-hint">Drag on the image to mark a detail · ← → next image · 1–5 weight · Esc closes</div>

        {item.analysis_status === 'error' ? (
          <div className="insp-status">
            <div className="error-box">{item.analysis_error || 'The analysis failed.'}</div>
            <button className="btn small" onClick={reanalyze}>
              <RefreshCw size={13} /> Analyze again
            </button>
          </div>
        ) : (
          <div className="insp-status muted small">{statusLine}</div>
        )}

        <section className="insp-section">
          <div className="field">
            <span>Weight</span>
            <div className="weights" role="group" aria-label="Weight">
              {[1, 2, 3, 4, 5].map((w) => (
                <button
                  key={w}
                  className={`weight-btn ${w < weight ? 'on' : ''} ${w === weight ? 'current' : ''}`}
                  aria-pressed={w === weight}
                  aria-label={`Weight ${w} of 5`}
                  onClick={() => setWeightNow(w)}
                >
                  {w}
                </button>
              ))}
            </div>
          </div>
          <label className="check">
            <input
              type="checkbox"
              checked={anti}
              onChange={(e) => {
                setAnti(e.target.checked);
                latest.current.anti = e.target.checked;
                saveAnnotation({ anti: e.target.checked });
              }}
            />
            Not this — the language must avoid what this image shows
          </label>
        </section>

        {regions.length > 0 && (
          <section className="insp-section">
            <h3>Marks</h3>
            {regions.map((r, i) => (
              <div key={r.key} className="mark-row">
                <span className="mark-label">r{i + 1}</span>
                <input
                  className="small-input"
                  placeholder="tag"
                  spellCheck={false}
                  autoCorrect="off"
                  autoCapitalize="off"
                  aria-label={`Tag of mark r${i + 1}`}
                  value={r.tag}
                  onChange={(e) => {
                    setRegions((rs) => rs.map((x) => (x.key === r.key ? { ...x, tag: e.target.value } : x)));
                    setRegionsDirty(true);
                  }}
                />
                <input
                  className="grow-input"
                  placeholder="What matters here?"
                  aria-label={`Note of mark r${i + 1}`}
                  value={r.note}
                  onChange={(e) => {
                    setRegions((rs) => rs.map((x) => (x.key === r.key ? { ...x, note: e.target.value } : x)));
                    setRegionsDirty(true);
                  }}
                />
                <button
                  className="icon-btn"
                  title="Remove mark"
                  aria-label={`Remove mark r${i + 1}`}
                  onClick={() => {
                    setRegions((rs) => rs.filter((x) => x.key !== r.key));
                    setRegionsDirty(true);
                  }}
                >
                  <X size={14} />
                </button>
              </div>
            ))}
          </section>
        )}
        {regionsDirty && (
          <div className="row">
            <button className="btn primary small" onClick={saveRegions}>
              Save marks
            </button>
            <button
              className="btn small"
              onClick={() => {
                setRegions(toDrafts(detail));
                setRegionsDirty(false);
              }}
            >
              Discard
            </button>
          </div>
        )}

        <section className="insp-section">
          <h3>Annotation</h3>
          <label className="field">
            <span>Tags (comma separated)</span>
            <input
              value={tags}
              onChange={(e) => {
                setTags(e.target.value);
                dirty.current = true;
              }}
              onBlur={saveIfDirty}
              placeholder="edge, grip, colour"
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
            />
          </label>
          <label className="field">
            <span>Note</span>
            <textarea
              rows={3}
              value={note}
              onChange={(e) => {
                setNote(e.target.value);
                dirty.current = true;
              }}
              onBlur={saveIfDirty}
              placeholder="Why is this image here?"
            />
          </label>
        </section>

        <section className="insp-section">
          <h3>Analysis</h3>
          {detail?.analysis ? (
            <AnalysisBlock analysis={detail.analysis} model={detail.analysis_model} />
          ) : (
            <div className="muted small">No analysis yet.</div>
          )}
          {item.analysis_status !== 'error' && (
            <div className="row">
              <button className="btn small" onClick={reanalyze}>
                <RefreshCw size={13} /> Analyze again
              </button>
            </div>
          )}
        </section>

        {item.palette.length > 0 && (
          <section className="insp-section">
            <h3>Colours</h3>
            <div className="swatch-row">
              {item.palette.map((c) => (
                <button
                  key={c.hex}
                  className="palette-swatch"
                  title={`${c.hex} · ${Math.round(c.share * 100)} % of the image. Click to copy.`}
                  onClick={() =>
                    navigator.clipboard
                      .writeText(c.hex)
                      .then(() => toast.info(`Copied ${c.hex}.`))
                      .catch(() => {})
                  }
                >
                  <span className="swatch-dot" style={{ background: c.hex }} />
                  <span className="mono">{c.hex}</span>
                </button>
              ))}
            </div>
            <div className="muted small">The main colours of the whole image, background included. The analysis separates product colours from the background.</div>
          </section>
        )}

        {item.model_rel && (
          <section className="insp-section">
            <h3>3D model</h3>
            <div className="small">
              The picture shows four automatic views of <span className="mono">{item.original_name}</span>: form and proportions, no colour or material.
            </div>
            <div className="row">
              <button
                className="btn small"
                onClick={() =>
                  api
                    .appInfo()
                    .then((i) => api.revealPath(`${i.library_path}/${item.model_rel!.split('/').slice(0, -1).join('/')}`))
                    .catch((e) => toast.error(errText(e)))
                }
              >
                Show the model file in Finder
              </button>
            </div>
          </section>
        )}

        <section className="insp-section">
          <h3>Source</h3>
          {src ? (
            isWeb ? (
              <button className="link-btn wrap" onClick={() => openUrl(src).catch((e) => toast.error(errText(e)))}>
                {src}
              </button>
            ) : (
              <div className="mono small wrap">{src}</div>
            )
          ) : (
            <div className="muted small">No source recorded.</div>
          )}
          <div className="muted small insp-meta">
            {item.width} × {item.height} px · added {fmtDate(item.added_at)}
          </div>
        </section>

        <section className="insp-section">
          {confirmRemove ? (
            <div className="row">
              <button
                className="btn danger small"
                onClick={() => {
                  if (removing.current) return;
                  // A removed item has no annotation or marks to save, also when it closes before the removal ends.
                  removing.current = true;
                  const wasDirty = dirty.current;
                  dirty.current = false;
                  api
                    .removeItem(item.id)
                    .then(onRemoved)
                    .catch((e) => {
                      removing.current = false;
                      dirty.current = dirty.current || wasDirty;
                      toast.error(errText(e));
                    });
                }}
              >
                <Trash2 size={13} /> Remove from language
              </button>
              <button className="btn small" onClick={cancelRemove} autoFocus>
                Cancel
              </button>
              <span className="muted small">The file moves to the _removed folder.</span>
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
