import { PointerEvent as RPointerEvent, ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LayoutGrid, Maximize, Minus, Plus } from 'lucide-react';
import { api, CanvasPlacement, CanvasPos, errText, fileUrl, Item } from '../api';
import { useToast } from './Toast';

/** Width of a newly placed card, in canvas units. */
const CARD_W = 240;
const GAP = 24;
const MIN_Z = 0.1;
const MAX_Z = 4;
/** Above this many screen pixels, the card shows the full image instead of the thumbnail. */
const FULL_RES_PX = 700;

interface View {
  /** Screen = canvas * z + t. */
  tx: number;
  ty: number;
  z: number;
}

interface Props {
  languageId: string;
  /** The items to show (already filtered by tag and "not this"). */
  items: Item[];
  selected: string | null;
  /** `byKey`: the keyboard chose it, so the inspector takes the focus. */
  onSelect: (id: string | null, byKey?: boolean) => void;
  /** The caption under each card (weight, status, tags). */
  renderCaption: (item: Item) => ReactNode;
  /** What a screen reader says for a card. */
  cardLabel: (item: Item) => string;
  /** Places were saved; the parent keeps its item list in step. */
  onPlaced: (places: CanvasPlacement[]) => void;
  /** Pan so that this card is in view (a link from Insight, an arrow key). A new object asks again. */
  revealReq?: { id: string } | null;
}

/** Share of the remaining distance that an animated zoom covers per frame. */
const EASE = 0.22;
/** Length of the glide after Arrange by weight (.canvas-world.arranging in styles.css). */
const ARRANGE_MS = 320;

const reducedMotion = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;

const aspect = (it: Item) => Math.max(1, it.height) / Math.max(1, it.width);

/** Masonry in `cols` columns, starting at (x0, y0). Returns the places in item order. */
function masonry(items: Item[], cols: number, x0: number, y0: number): CanvasPlacement[] {
  const heights = new Array(Math.max(1, cols)).fill(y0);
  return items.map((it) => {
    const c = heights.indexOf(Math.min(...heights));
    const place = { id: it.id, x: x0 + c * (CARD_W + GAP), y: heights[c], w: CARD_W };
    heights[c] += CARD_W * aspect(it) + GAP;
    return place;
  });
}

function bounds(items: Item[], pos: Record<string, CanvasPos>) {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const it of items) {
    const p = pos[it.id];
    if (!p) continue;
    minX = Math.min(minX, p.x);
    minY = Math.min(minY, p.y);
    maxX = Math.max(maxX, p.x + p.w);
    maxY = Math.max(maxY, p.y + p.w * aspect(it));
  }
  return Number.isFinite(minX) ? { minX, minY, maxX, maxY } : null;
}

function loadView(languageId: string): View | null {
  try {
    const v = JSON.parse(localStorage.getItem(`cory.canvasView.${languageId}`) ?? 'null');
    return v && Number.isFinite(v.tx) && Number.isFinite(v.ty) && Number.isFinite(v.z) ? v : null;
  } catch {
    return null;
  }
}

function saveView(languageId: string, v: View) {
  try {
    localStorage.setItem(`cory.canvasView.${languageId}`, JSON.stringify(v));
  } catch {
    // Storage is a convenience only.
  }
}

type Gesture =
  | { kind: 'pan'; sx: number; sy: number; tx: number; ty: number }
  | { kind: 'move'; id: string; sx: number; sy: number; x: number; y: number; moved: boolean }
  | { kind: 'resize'; id: string; sx: number; w: number };

export default function CanvasView({ languageId, items, selected, onSelect, renderCaption, cardLabel, onPlaced, revealReq = null }: Props) {
  const toast = useToast();
  const wrapRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<Record<string, CanvasPos>>({});
  const [view, setView] = useState<View>(() => loadView(languageId) ?? { tx: 40, ty: 40, z: 0.8 });
  const gesture = useRef<Gesture | null>(null);
  const fitted = useRef(loadView(languageId) !== null);
  const [panning, setPanning] = useState(false);
  // True for a moment after Arrange by weight (and its Undo): the cards glide to
  // their new places. At other times a card's place follows the pointer at once.
  const [arranging, setArranging] = useState(false);
  const arrangeTimer = useRef<number | undefined>(undefined);
  const glide = useCallback(() => {
    setArranging(true);
    window.clearTimeout(arrangeTimer.current);
    arrangeTimer.current = window.setTimeout(() => setArranging(false), ARRANGE_MS + 50);
  }, []);
  useEffect(() => () => window.clearTimeout(arrangeTimer.current), []);

  const save = useCallback(
    (places: CanvasPlacement[]) => {
      if (places.length === 0) return;
      onPlaced(places);
      api.setCanvas(languageId, places).catch((e) => toast.error(errText(e)));
    },
    [languageId, toast, onPlaced],
  );

  // Local places win (they may be newer than a reload that raced a save); saved
  // places fill in the rest; new items go below the others and are saved.
  const posRef = useRef(pos);
  posRef.current = pos;
  useEffect(() => {
    const old = posRef.current;
    const next: Record<string, CanvasPos> = {};
    for (const it of items) {
      const p = old[it.id] ?? it.canvas;
      if (p) next[it.id] = p;
    }
    const unplaced = items.filter((it) => !next[it.id]);
    if (unplaced.length > 0) {
      const b = bounds(items, next);
      const cols = b
        ? Math.max(3, Math.round((b.maxX - b.minX + GAP) / (CARD_W + GAP)))
        : Math.max(3, Math.round(Math.sqrt(unplaced.length) * 1.3));
      const places = masonry(unplaced, cols, b ? b.minX : 0, b ? b.maxY + GAP * 3 : 0);
      for (const p of places) next[p.id] = { x: p.x, y: p.y, w: p.w };
      save(places);
    }
    setPos(next);
  }, [items, save]);

  useEffect(() => saveView(languageId, view), [languageId, view]);

  // Zoom eases toward a target view; panning sets the view at once and stops it.
  const viewRef = useRef(view);
  viewRef.current = view;
  const targetRef = useRef<View | null>(null);
  const rafRef = useRef<number | null>(null);
  const step = useCallback(() => {
    const t = targetRef.current;
    const v = viewRef.current;
    if (!t) {
      rafRef.current = null;
      return;
    }
    const done = Math.abs(t.z - v.z) / t.z < 0.002 && Math.abs(t.tx - v.tx) < 0.5 && Math.abs(t.ty - v.ty) < 0.5;
    const n = done ? t : { z: v.z + (t.z - v.z) * EASE, tx: v.tx + (t.tx - v.tx) * EASE, ty: v.ty + (t.ty - v.ty) * EASE };
    viewRef.current = n;
    setView(n);
    if (done) {
      targetRef.current = null;
      rafRef.current = null;
    } else {
      rafRef.current = requestAnimationFrame(step);
    }
  }, []);
  const animateTo = useCallback(
    (t: View) => {
      if (reducedMotion()) {
        targetRef.current = null;
        // Set the ref now too: a reveal in the same commit (after a fit) builds on this view.
        viewRef.current = t;
        setView(t);
        return;
      }
      targetRef.current = t;
      if (rafRef.current === null) rafRef.current = requestAnimationFrame(step);
    },
    [step],
  );
  const stopAnimation = useCallback(() => {
    targetRef.current = null;
    if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    rafRef.current = null;
  }, []);
  useEffect(() => stopAnimation, [stopAnimation]);

  const fit = useCallback(() => {
    const el = wrapRef.current;
    const b = bounds(items, pos);
    if (!el || !b) return;
    const pad = 48;
    const w = el.clientWidth - pad * 2;
    const h = el.clientHeight - pad * 2;
    const z = Math.min(MAX_Z, Math.max(MIN_Z, Math.min(w / (b.maxX - b.minX), h / (b.maxY - b.minY))));
    animateTo({
      z,
      tx: pad + (w - (b.maxX - b.minX) * z) / 2 - b.minX * z,
      ty: pad + (h - (b.maxY - b.minY) * z) / 2 - b.minY * z,
    });
  }, [items, pos, animateTo]);

  /** Zooms by `factor` around the screen point (sx, sy). Repeated calls add up
   *  on the target, so fast scrolling does not lose steps. */
  const zoomAt = useCallback(
    (factor: number, sx: number, sy: number, animate: boolean) => {
      const base = targetRef.current ?? viewRef.current;
      const z = Math.min(MAX_Z, Math.max(MIN_Z, base.z * factor));
      const k = z / base.z;
      const next = { z, tx: sx - (sx - base.tx) * k, ty: sy - (sy - base.ty) * k };
      if (animate) animateTo(next);
      else {
        stopAnimation();
        viewRef.current = next;
        setView(next);
      }
    },
    [animateTo, stopAnimation],
  );

  const zoomCenter = (factor: number) => {
    const el = wrapRef.current;
    if (el) zoomAt(factor, el.clientWidth / 2, el.clientHeight / 2, true);
  };

  // Trackpad: two fingers pan, a pinch (ctrlKey) or ⌘-scroll zooms at the pointer.
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = el.getBoundingClientRect();
      const sx = e.clientX - r.left;
      const sy = e.clientY - r.top;
      // Lines or pages (some mice) count as about 16 px each.
      const dy = e.deltaMode === 0 ? e.deltaY : e.deltaY * 16;
      if (e.ctrlKey) {
        // Trackpad pinch: many small steps; follow the fingers directly, gently.
        zoomAt(Math.exp(-Math.max(-30, Math.min(30, dy)) * 0.006), sx, sy, false);
      } else if (e.metaKey) {
        // ⌘ + mouse wheel: large steps; capped and eased.
        zoomAt(Math.exp(-Math.max(-60, Math.min(60, dy)) * 0.0025), sx, sy, true);
      } else {
        stopAnimation();
        setView((v) => ({ ...v, tx: v.tx - e.deltaX, ty: v.ty - e.deltaY }));
      }
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, [zoomAt, stopAnimation]);

  const onPointerDown = (e: RPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    const card = target.closest<HTMLElement>('[data-item]');
    const id = card?.dataset.item;
    if (target.closest('.canvas-tools')) return;
    stopAnimation();
    if (target.dataset.handle && id && pos[id]) {
      gesture.current = { kind: 'resize', id, sx: e.clientX, w: pos[id].w };
    } else if (id && pos[id]) {
      gesture.current = { kind: 'move', id, sx: e.clientX, sy: e.clientY, x: pos[id].x, y: pos[id].y, moved: false };
    } else {
      gesture.current = { kind: 'pan', sx: e.clientX, sy: e.clientY, tx: viewRef.current.tx, ty: viewRef.current.ty };
      setPanning(true);
    }
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: RPointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    if (!g) return;
    if (g.kind === 'pan') {
      setView((v) => ({ ...v, tx: g.tx + e.clientX - g.sx, ty: g.ty + e.clientY - g.sy }));
    } else if (g.kind === 'move') {
      const dx = (e.clientX - g.sx) / view.z;
      const dy = (e.clientY - g.sy) / view.z;
      if (!g.moved && Math.hypot(e.clientX - g.sx, e.clientY - g.sy) < 4) return;
      g.moved = true;
      setPos((p) => ({ ...p, [g.id]: { ...p[g.id], x: g.x + dx, y: g.y + dy } }));
    } else {
      const w = Math.min(4000, Math.max(40, g.w + (e.clientX - g.sx) / view.z));
      setPos((p) => ({ ...p, [g.id]: { ...p[g.id], w } }));
    }
  };

  const onPointerUp = (e: RPointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    gesture.current = null;
    setPanning(false);
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    if (!g) return;
    if (g.kind === 'pan') {
      // A click on the background closes the inspector.
      if (Math.hypot(e.clientX - g.sx, e.clientY - g.sy) < 4) onSelect(null);
    } else if (g.kind === 'move' && !g.moved) {
      onSelect(g.id === selected ? null : g.id);
    } else {
      const p = pos[g.id];
      if (p) save([{ id: g.id, ...p }]);
    }
  };

  const tidy = () => {
    const before: CanvasPlacement[] = items.filter((it) => pos[it.id]).map((it) => ({ id: it.id, ...pos[it.id] }));
    const order = [...items].sort((a, b) => b.weight - a.weight || b.added_at.localeCompare(a.added_at));
    const cols = Math.max(3, Math.round(Math.sqrt(order.length) * 1.3));
    const places = masonry(order, cols, 0, 0);
    glide();
    setPos((p) => {
      const next = { ...p };
      for (const pl of places) next[pl.id] = { x: pl.x, y: pl.y, w: pl.w };
      return next;
    });
    save(places);
    fitted.current = false;
    toast.info('Arranged by weight.', {
      label: 'Undo',
      run: () => {
        glide();
        setPos((p) => {
          const next = { ...p };
          for (const pl of before) next[pl.id] = { x: pl.x, y: pl.y, w: pl.w };
          return next;
        });
        save(before);
      },
    });
  };

  /** Pans (at the same zoom) so that a card the keyboard reached is in view. */
  const reveal = useCallback(
    (id: string) => {
      const el = wrapRef.current;
      const p = pos[id];
      const it = items.find((x) => x.id === id);
      if (!el || !p || !it) return;
      const v = targetRef.current ?? viewRef.current;
      const pad = 24;
      const x0 = p.x * v.z + v.tx;
      const y0 = p.y * v.z + v.ty;
      const x1 = (p.x + p.w) * v.z + v.tx;
      const y1 = (p.y + p.w * aspect(it)) * v.z + v.ty;
      const shift = (a: number, b: number, size: number) => (a < pad ? pad - a : b > size - pad ? Math.max(pad - a, size - pad - b) : 0);
      const dx = shift(x0, x1, el.clientWidth);
      const dy = shift(y0, y1, el.clientHeight);
      if (dx || dy) animateTo({ ...v, tx: v.tx + dx, ty: v.ty + dy });
    },
    [items, pos, animateTo],
  );

  // Fit on the first open of a language, and after a tidy, once the places are in.
  useEffect(() => {
    if (!fitted.current && Object.keys(pos).length > 0) {
      fitted.current = true;
      fit();
    }
  }, [pos, fit]);

  // A card that the board asks for: pan to it once its place is known (after the first fit, which runs above).
  const revealed = useRef<{ id: string } | null>(null);
  useEffect(() => {
    if (!revealReq || revealed.current === revealReq || !pos[revealReq.id]) return;
    revealed.current = revealReq;
    reveal(revealReq.id);
  }, [revealReq, pos, reveal]);

  const cards = useMemo(
    () =>
      items
        .filter((it) => pos[it.id])
        .map((it) => {
          const p = pos[it.id];
          const screenW = p.w * view.z;
          const src = screenW > FULL_RES_PX ? fileUrl(it.file_rel) : fileUrl(it.thumb_rel);
          return (
            <figure
              key={it.id}
              data-item={it.id}
              data-card={it.id}
              tabIndex={0}
              role="button"
              aria-pressed={selected === it.id}
              aria-label={cardLabel(it)}
              className={`card canvas-card ${it.anti ? 'anti' : ''} ${selected === it.id ? 'selected' : ''}`}
              style={{ transform: `translate(${p.x}px, ${p.y}px)`, width: p.w }}
              onKeyDown={(e) => {
                if (e.target !== e.currentTarget || e.repeat || (e.key !== 'Enter' && e.key !== ' ')) return;
                e.preventDefault();
                onSelect(it.id === selected ? null : it.id, it.id !== selected);
              }}
              onFocus={(e) => {
                // Only a keyboard focus pans; a click on a card at the edge must not move the board under the pointer.
                if (e.currentTarget.matches(':focus-visible')) reveal(it.id);
              }}
            >
              <img src={src} alt={it.original_name} draggable={false} style={{ aspectRatio: `${Math.max(1, it.width)} / ${Math.max(1, it.height)}` }} />
              {it.anti && <span className="anti-badge">Not this</span>}
              {screenW > 140 && <figcaption>{renderCaption(it)}</figcaption>}
              <span className="canvas-handle" data-handle="1" title="Drag to resize" />
            </figure>
          );
        }),
    [items, pos, view.z, selected, renderCaption, cardLabel, onSelect, reveal],
  );

  return (
    <div
      ref={wrapRef}
      className={`canvas-wrap ${panning ? 'panning' : ''}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onScroll={(e) => {
        // WebKit scrolls this clipped box to show a focused card; the view transform does the panning.
        const el = e.currentTarget;
        if (el.scrollLeft || el.scrollTop) {
          el.scrollLeft = 0;
          el.scrollTop = 0;
        }
      }}
    >
      <div
        className={`canvas-world ${arranging ? 'arranging' : ''}`}
        style={{ transform: `translate(${view.tx}px, ${view.ty}px) scale(${view.z})`, ['--inv-z' as string]: 1 / view.z }}
      >
        {cards}
      </div>
      <div className="canvas-tools" role="toolbar" aria-label="Canvas">
        <button className="icon-btn" onClick={() => zoomCenter(1 / 1.25)} title="Zoom out" aria-label="Zoom out">
          <Minus size={14} />
        </button>
        <button className="canvas-zoom" onClick={() => zoomCenter(1 / (targetRef.current ?? viewRef.current).z)} title="Zoom to 100 %">
          {Math.round(view.z * 100)} %
        </button>
        <button className="icon-btn" onClick={() => zoomCenter(1.25)} title="Zoom in" aria-label="Zoom in">
          <Plus size={14} />
        </button>
        <span className="canvas-sep" />
        <button className="icon-btn" onClick={fit} title="Fit all" aria-label="Fit all">
          <Maximize size={14} />
        </button>
        <button className="icon-btn" onClick={tidy} title="Arrange by weight" aria-label="Arrange by weight">
          <LayoutGrid size={14} />
        </button>
      </div>
      <div className="canvas-help muted small">Drag the background to pan · Pinch or ⌘-scroll to zoom · Drag a card to move it, its corner to resize</div>
    </div>
  );
}
