import { PointerEvent, useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { api, errText, fileUrl, fmtDate, Item, ItemDetail, RegionInput, shortId } from '../api';
import { RefreshCw, Trash2, X } from 'lucide-react';
import { useToast } from './Toast';

interface Props {
  item: Item;
  tick: number;
  onClose: () => void;
  onChanged: () => void;
  onRemoved: () => void;
}

type Draft = RegionInput & { key: string };

let keySeq = 0;
const newKey = () => `k${++keySeq}`;

function toDrafts(d: ItemDetail | null): Draft[] {
  return (d?.regions ?? []).map((r) => ({ ...r, id: r.id, key: newKey() }));
}

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

export default function ItemInspector({ item, tick, onClose, onChanged, onRemoved }: Props) {
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
  const frameRef = useRef<HTMLDivElement>(null);

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

  const saveAnnotation = async (over?: Partial<{ weight: number; anti: boolean }>) => {
    try {
      const tagList = tags
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean);
      await api.updateItem(item.id, tagList, note, over?.weight ?? weight, over?.anti ?? anti);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const saveRegions = async () => {
    try {
      await api.setRegions(
        item.id,
        regions.map(({ id, x, y, w, h, note: n, tag }) => ({ id, x, y, w, h, note: n, tag })),
      );
      setRegionsDirty(false);
      toast.info('Marks saved. The item gets analyzed again with the crops.');
      onChanged();
    } catch (e) {
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

  return (
    <aside className="inspector">
      <div className="insp-head">
        <div>
          <div className="eyebrow">Item {shortId(item.id)}</div>
          <div className="insp-name" title={item.original_name}>
            {item.original_name}
          </div>
        </div>
        <button className="icon-btn" onClick={onClose} title="Close">
          <X size={15} />
        </button>
      </div>

      <div className="insp-scroll">
        <div className="frame" ref={frameRef}>
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
        <div className="frame-hint">
          Drag on the image to mark a detail. {item.width} × {item.height} px · added {fmtDate(item.added_at)}
        </div>

        {regions.length > 0 && (
          <section className="insp-section">
            <h3>Marks</h3>
            {regions.map((r, i) => (
              <div key={r.key} className="mark-row">
                <span className="mark-label">r{i + 1}</span>
                <input
                  className="small-input"
                  placeholder="tag"
                  value={r.tag}
                  onChange={(e) => {
                    setRegions((rs) => rs.map((x) => (x.key === r.key ? { ...x, tag: e.target.value } : x)));
                    setRegionsDirty(true);
                  }}
                />
                <input
                  className="grow-input"
                  placeholder="What matters here?"
                  value={r.note}
                  onChange={(e) => {
                    setRegions((rs) => rs.map((x) => (x.key === r.key ? { ...x, note: e.target.value } : x)));
                    setRegionsDirty(true);
                  }}
                />
                <button
                  className="icon-btn"
                  title="Remove mark"
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
            <input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="edge, grip, colour" />
          </label>
          <label className="field">
            <span>Note</span>
            <textarea rows={3} value={note} onChange={(e) => setNote(e.target.value)} placeholder="Why is this image here?" />
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
                    saveAnnotation({ weight: w });
                  }}
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
                saveAnnotation({ anti: e.target.checked });
              }}
            />
            Not this — the language must avoid what this image shows
          </label>
          {(tags !== item.tags.join(', ') || note !== item.note) && (
            <div className="row">
              <button className="btn primary small" onClick={() => saveAnnotation()}>
                Save annotation
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
        </section>

        <section className="insp-section">
          <h3>
            Analysis <span className={`dot status-${item.analysis_status}`} />
          </h3>
          {item.analysis_status === 'error' && <div className="error-box">{item.analysis_error}</div>}
          {item.analysis_status === 'pending' && <div className="muted small">In the queue.</div>}
          {item.analysis_status === 'analyzing' && <div className="muted small">The model reads this image now.</div>}
          {detail?.analysis && <AnalysisBlock analysis={detail.analysis} model={detail.analysis_model} />}
          <div className="row">
            <button
              className="btn small"
              onClick={() =>
                api
                  .reanalyzeItem(item.id)
                  .then(onChanged)
                  .catch((e) => toast.error(errText(e)))
              }
            >
              <RefreshCw size={13} /> Analyze again
            </button>
          </div>
        </section>

        <section className="insp-section">
          {confirmRemove ? (
            <div className="row">
              <button
                className="btn danger small"
                onClick={() =>
                  api
                    .removeItem(item.id)
                    .then(onRemoved)
                    .catch((e) => toast.error(errText(e)))
                }
              >
                <Trash2 size={13} /> Remove from language
              </button>
              <button className="btn small" onClick={() => setConfirmRemove(false)}>
                Cancel
              </button>
              <span className="muted small">The file moves to the _removed folder.</span>
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
