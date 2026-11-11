import { DragEvent, FormEvent, KeyboardEvent as ReactKeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { listen } from '@tauri-apps/api/event';
import { CircleAlert, Grid2x2, ImagePlus, Import, Loader, Move, Search, X } from 'lucide-react';
import {
  api,
  BoardHit,
  CanvasPlacement,
  DOC_EXT_RE,
  errText,
  fileToBase64,
  fileUrl,
  fmtDuration,
  IMAGE_URL_RE,
  Item,
  Language,
  MODEL_EXT_RE,
  MODEL_EXTENSIONS,
  Swatch,
  VIDEO_EXT_RE,
  VIDEO_EXTENSIONS,
  VideoNote,
} from '../api';
import ItemInspector from './ItemInspector';
import CanvasView from './CanvasView';
import { useToast } from './Toast';

const IMAGE_EXT = /\.(png|jpe?g|webp|gif|bmp|tiff?)$/i;

function firstUri(list: string): string | null {
  const line = list
    .split(/\r?\n/)
    .map((l) => l.trim())
    .find((l) => l && !l.startsWith('#'));
  return line ?? null;
}

function imgSrcFromHtml(html: string): string | null {
  if (!html) return null;
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const img = doc.querySelector('img');
  const src = img?.getAttribute('src') ?? null;
  return src && /^https?:\/\//.test(src) ? src : null;
}

type BoardMode = 'grid' | 'canvas';

function loadMode(languageId: string): BoardMode {
  try {
    return localStorage.getItem(`cory.boardMode.${languageId}`) === 'canvas' ? 'canvas' : 'grid';
  } catch {
    return 'grid';
  }
}

/** Where new images wait: in Manual mode for Run queue, or behind a paused queue. */
export type Waits = 'manual' | 'paused' | null;

function Caption({ item }: { item: Item }) {
  return (
    <>
      <span className="weight" role="img" aria-label={`Weight ${item.weight} of 5`}>
        {[1, 2, 3, 4, 5].map((n) => (
          <i key={n} className={n <= item.weight ? 'on' : ''} />
        ))}
      </span>
      <StatusLabel item={item} />
      {item.region_count > 0 && <span className="cap-meta">{item.region_count} mark{item.region_count === 1 ? '' : 's'}</span>}
      <span className="cap-tags">{item.tags.slice(0, 3).join(' · ')}</span>
    </>
  );
}

const renderCaption = (item: Item) => <Caption item={item} />;

/** Nothing when the analysis is done; a word (and an icon) for every other state. */
function StatusLabel({ item }: { item: Item }) {
  switch (item.analysis_status) {
    case 'done':
      return null;
    case 'error':
      return (
        <span className="cap-status cap-error" title={`Analysis failed: ${item.analysis_error ?? ''}`}>
          <CircleAlert size={11} aria-hidden="true" /> Failed
        </span>
      );
    case 'analyzing':
      return (
        <span className="cap-status cap-busy">
          <Loader size={11} className="spin" aria-hidden="true" /> Analyzing
        </span>
      );
    default:
      return <span className="cap-status">Waiting</span>;
  }
}

const STATUS_TEXT: Record<string, string> = {
  done: 'Analyzed',
  error: 'Analysis failed',
  analyzing: 'Analyzing',
};

/** What a screen reader says for a card. */
export function cardLabel(item: Item): string {
  const status = STATUS_TEXT[item.analysis_status] ?? 'Waiting for analysis';
  return (
    `${item.original_name}. ${status}. Weight ${item.weight} of 5.` +
    (item.anti ? ' Not this.' : '') +
    (item.kind === 'model' ? ' 3D model.' : '') +
    (item.tags.length > 0 ? ` Tags: ${item.tags.join(', ')}.` : '')
  );
}

const cardEl = (id: string) => document.querySelector<HTMLElement>(`[data-card="${CSS.escape(id)}"]`);

/** Keys in these places belong to the field, not to the board. */
const isField = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || !!t.closest('input, textarea, select'));

export default function Board({
  language,
  tick,
  onChanged,
  onSources,
  waits = null,
  focus = null,
}: {
  language: Language;
  tick: number;
  onChanged: () => void;
  onSources: () => void;
  /** Why new images do not start their analysis at once, if they do not. */
  waits?: Waits;
  /** Select the first item whose ID starts with this (a link from Insight). */
  focus?: string | null;
}) {
  const toast = useToast();
  const [items, setItems] = useState<Item[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState(0);
  const [filter, setFilter] = useState('');
  const [showAnti, setShowAnti] = useState(true);
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<BoardHit[] | null>(null);
  const [palette, setPalette] = useState<Swatch[]>([]);
  const [boardUrl, setBoardUrl] = useState<string | null>(null);
  const [boardBusy, setBoardBusy] = useState<string | null>(null);
  const [mode, setModeState] = useState<BoardMode>(() => loadMode(language.id));
  /** Canvas: the card to pan into view. A new object asks again (scrollIntoView does nothing there). */
  const [revealReq, setRevealReq] = useState<{ id: string } | null>(null);
  const setMode = (m: BoardMode) => {
    setModeState(m);
    setRevealReq(null);
    try {
      localStorage.setItem(`cory.boardMode.${language.id}`, m);
    } catch {
      // Storage is a convenience only.
    }
  };
  const dragDepth = useRef(0);

  const load = useCallback(() => {
    api
      .listItems(language.id)
      .then((list) => {
        setItems(list);
        setLoaded(true);
      })
      .catch((e) => toast.error(errText(e)));
  }, [language.id, toast]);

  useEffect(load, [load, tick]);

  // Images of older versions have no palette yet: find their colours one time.
  useEffect(() => {
    let alive = true;
    api
      .backfillPalettes(language.id)
      .then((n) => alive && n > 0 && load())
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [language.id, load]);

  // The palette of the board follows the items.
  useEffect(() => {
    let alive = true;
    api
      .languagePalette(language.id)
      .then((p) => alive && setPalette(p))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [language.id, items]);

  // Search: the same search that agents use, a moment after the last key.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setHits(null);
      return;
    }
    let alive = true;
    const t = setTimeout(() => {
      api
        .searchBoard(language.id, q)
        .then((h) => alive && setHits(h))
        .catch((e) => alive && toast.error(errText(e)));
    }, 180);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [query, language.id, items, toast]);

  // Progress of a board import.
  useEffect(() => {
    const un = listen<{ done: number; total: number }>('board-import-progress', (e) =>
      setBoardBusy(`Importing ${e.payload.done} of ${e.payload.total}…`),
    );
    return () => {
      un.then((f) => f());
    };
  }, []);

  const importBoard = async (e: FormEvent) => {
    e.preventDefault();
    const url = (boardUrl ?? '').trim();
    if (!url || boardBusy) return;
    setBoardBusy('Reading the board…');
    try {
      const preview = await api.previewBoard(url);
      if (preview.items.length === 0) {
        toast.error(`“${preview.title}” has no images to import.`);
        return;
      }
      setBoardBusy(`Importing 0 of ${preview.items.length}…`);
      const r = await api.importBoard(language.id, url);
      r.errors.slice(0, 3).forEach((x) => toast.error(x));
      toast.info(
        `${r.added} image${r.added === 1 ? '' : 's'} from “${preview.title}” (${preview.service}) added.` +
          (r.duplicates > 0 ? ` ${r.duplicates} were here already.` : '') +
          (r.errors.length > 3 ? ` ${r.errors.length} could not be read.` : '') +
          (preview.note ? ` ${preview.note}` : ''),
      );
      setBoardUrl(null);
      load();
      onChanged();
    } catch (err) {
      toast.error(errText(err));
    } finally {
      setBoardBusy(null);
    }
  };

  const afterImport = useCallback(
    (added: number, errors: string[], videos: VideoNote[] = []) => {
      const next = (n: number) =>
        waits === 'manual' ? (n === 1 ? 'It waits for Run queue.' : 'They wait for Run queue.') : waits === 'paused' ? 'The queue is paused.' : 'The analysis starts now.';
      errors.forEach((e) => toast.error(e));
      for (const v of videos) {
        toast.info(
          v.frames > 0
            ? `${v.frames} key frame${v.frames === 1 ? '' : 's'} from “${v.name}” (${fmtDuration(v.duration * 1000)}) added. ${next(v.frames)}`
            : `No usable frames in “${v.name}”.`,
        );
      }
      const stills = added - videos.reduce((n, v) => n + v.frames, 0);
      if (stills > 0) toast.info(`${stills} image${stills === 1 ? '' : 's'} added. ${next(stills)}`);
      load();
      onChanged();
    },
    [load, onChanged, toast, waits],
  );

  const importFiles = useCallback(
    async (files: File[], source: string | null) => {
      let added = 0;
      const errors: string[] = [];
      const videos: VideoNote[] = [];
      setBusy((b) => b + 1);
      try {
        let docs = 0;
        for (const f of files) {
          if (DOC_EXT_RE.test(f.name)) {
            // Documents go to Sources.
            try {
              await api.importSourceBytes(language.id, f.name, await fileToBase64(f));
              docs++;
            } catch (e) {
              errors.push(errText(e));
            }
            continue;
          }
          if (MODEL_EXT_RE.test(f.name)) {
            // A 3D model: the app draws four views of it.
            toast.info(`Drawing the views of “${f.name}”…`);
            try {
              await api.importModelBytes(language.id, f);
              added++;
            } catch (e) {
              errors.push(errText(e));
            }
            continue;
          }
          if (VIDEO_EXT_RE.test(f.name) || f.type.startsWith('video/')) {
            toast.info(`Reading key frames from “${f.name}”…`);
            try {
              const v = await api.importVideoBytes(language.id, f);
              videos.push({ name: f.name, frames: v.items.length, duration: v.duration });
              added += v.items.length;
            } catch (e) {
              errors.push(errText(e));
            }
            continue;
          }
          if (!f.type.startsWith('image/') && !IMAGE_EXT.test(f.name)) {
            errors.push(`"${f.name}" is not an image.`);
            continue;
          }
          try {
            const data = await fileToBase64(f);
            await api.importBytes(language.id, f.name || 'pasted.png', data, files.length === 1 ? source : null);
            added++;
          } catch (e) {
            errors.push(errText(e));
          }
        }
        if (docs > 0) toast.info(`${docs} document${docs === 1 ? '' : 's'} added to Sources.`);
      } finally {
        setBusy((b) => b - 1);
      }
      afterImport(added, errors, videos);
    },
    [afterImport, language.id, toast],
  );

  // A link to a page, video or document becomes a source.
  const addLinkSource = useCallback(
    async (url: string) => {
      setBusy((b) => b + 1);
      try {
        const s = await api.addLink(language.id, url);
        toast.info(`“${s.title}” added to Sources. The model reads it next.`, { label: 'Open Sources', run: onSources });
        onChanged();
      } catch (err) {
        // Some image links have no file extension: try them as an image.
        try {
          await api.importUrl(language.id, url, url);
          afterImport(1, []);
        } catch {
          toast.error(errText(err));
        }
      } finally {
        setBusy((b) => b - 1);
      }
    },
    [afterImport, language.id, onChanged, onSources, toast],
  );

  const onDrop = async (e: DragEvent) => {
    e.preventDefault();
    dragDepth.current = 0;
    setDragging(false);
    const dt = e.dataTransfer;
    const files = Array.from(dt.files);
    const uriList = dt.getData('text/uri-list');
    const html = dt.getData('text/html');
    const uri = firstUri(uriList);
    const webSource = uri && /^https?:\/\//.test(uri) ? uri : null;

    if (files.length > 0) {
      // Files from Finder: the source is the file name. Files from a browser: the link.
      await importFiles(files, webSource ?? (files.length === 1 ? files[0].name : null));
      return;
    }
    const htmlImg = imgSrcFromHtml(html);
    if (!htmlImg && webSource && !IMAGE_URL_RE.test(webSource)) {
      await addLinkSource(webSource);
      return;
    }
    const imgUrl = htmlImg ?? webSource;
    if (imgUrl) {
      setBusy((b) => b + 1);
      try {
        await api.importUrl(language.id, imgUrl, webSource ?? imgUrl);
        afterImport(1, []);
      } catch (err) {
        afterImport(0, [errText(err)]);
      } finally {
        setBusy((b) => b - 1);
      }
      return;
    }
    toast.error('The drop had no image or link. Drag an image file, a document, or an image or link from a browser.');
  };

  // Paste a screenshot (Cmd+Ctrl+Shift+4 copies one) or an image with Cmd+V.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA')) return;
      const files = Array.from(e.clipboardData?.files ?? []);
      if (files.length > 0) {
        e.preventDefault();
        importFiles(files, 'clipboard');
        return;
      }
      const text = e.clipboardData?.getData('text/plain')?.trim() ?? '';
      if (/^https?:\/\/\S+$/.test(text) && !IMAGE_URL_RE.test(text)) {
        e.preventDefault();
        addLinkSource(text);
        return;
      }
      if (/^https?:\/\/\S+$/.test(text)) {
        e.preventDefault();
        setBusy((b) => b + 1);
        api
          .importUrl(language.id, text, text)
          .then(() => afterImport(1, []))
          .catch((err) => afterImport(0, [errText(err)]))
          .finally(() => setBusy((b) => b - 1));
      }
    };
    document.addEventListener('paste', onPaste);
    return () => document.removeEventListener('paste', onPaste);
  }, [addLinkSource, afterImport, importFiles, language.id]);

  const addFromDialog = async () => {
    const picked = await open({
      multiple: true,
      title: 'Add images, videos or 3D models',
      filters: [
        {
          name: 'Images, videos and 3D models',
          extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'tif', 'tiff', ...VIDEO_EXTENSIONS, ...MODEL_EXTENSIONS],
        },
      ],
    });
    const paths = Array.isArray(picked) ? picked : typeof picked === 'string' ? [picked] : [];
    if (paths.length === 0) return;
    if (paths.some((p) => VIDEO_EXT_RE.test(p))) toast.info('Reading key frames from the video…');
    if (paths.some((p) => MODEL_EXT_RE.test(p))) toast.info('Drawing the views of the 3D model…');
    setBusy((b) => b + 1);
    try {
      const r = await api.importFiles(language.id, paths);
      afterImport(r.added.length, r.errors, r.videos);
    } catch (e) {
      afterImport(0, [errText(e)]);
    } finally {
      setBusy((b) => b - 1);
    }
  };

  const allTags = useMemo(() => {
    const s = new Set<string>();
    items.forEach((i) => i.tags.forEach((t) => s.add(t)));
    return Array.from(s).sort();
  }, [items]);

  const shown = useMemo(() => {
    const base = items.filter((i) => (showAnti || !i.anti) && (!filter || i.tags.includes(filter)));
    if (!hits) return base;
    // With a search: only the matches, best first.
    const rank = new Map(hits.map((h, n) => [h.item_id, n]));
    return base.filter((i) => rank.has(i.id)).sort((a, b) => rank.get(a.id)! - rank.get(b.id)!);
  }, [items, showAnti, filter, hits]);
  const hitOf = useMemo(() => new Map((hits ?? []).map((h) => [h.item_id, h])), [hits]);
  const selectedItem = items.find((i) => i.id === selected) ?? null;
  /** The selection now, for callbacks that end after other keys or clicks changed it. */
  const selectedRef = useRef(selected);
  selectedRef.current = selected;

  // Keyboard: the inspector takes focus when a key opened it; a card takes it back on close.
  const openedByKey = useRef(false);
  const wantFocus = useRef<string | null>(null);
  const scrollReq = useRef<{ id: string; block: ScrollLogicalPosition } | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const select = useCallback((id: string | null, byKey = false) => {
    openedByKey.current = byKey && id !== null;
    setSelected(id);
  }, []);

  /** Focus a card (grid: and scroll it into view). Done after the render, so a new layout does not lose it. */
  const focusCard = useCallback(
    (id: string) => {
      wantFocus.current = id;
      requestAnimationFrame(() => {
        const el = cardEl(id);
        if (!el) return;
        el.focus({ preventScroll: true });
        if (mode === 'grid') el.scrollIntoView({ block: 'nearest' });
      });
    },
    [mode],
  );

  /** Focus goes back to the board when it was in the inspector (or nowhere). */
  const focusWasInInspector = () => {
    const a = document.activeElement;
    return !a || a === document.body || !!a.closest('.inspector');
  };

  const closeInspector = useCallback(() => {
    const id = selected;
    const back = focusWasInInspector();
    setSelected(null);
    if (id && back) focusCard(id);
  }, [selected, focusCard]);

  // The canvas saved new places: keep the item list in step, so a switch to Grid
  // and back (or any re-render) starts from the saved places.
  const onPlaced = useCallback((places: CanvasPlacement[]) => {
    const byId = new Map(places.map((p) => [p.id, { x: p.x, y: p.y, w: p.w }]));
    setItems((list) => list.map((i) => (byId.has(i.id) ? { ...i, canvas: byId.get(i.id)! } : i)));
  }, []);

  // Masonry: put each card in the shortest column. The image sizes are known, so the layout is stable.
  // The column count follows the whole board, so opening the inspector narrows the cards
  // instead of moving them to other columns (down to 130 px a card).
  const [wrapEl, setWrapEl] = useState<HTMLDivElement | null>(null);
  const [gridEl, setGridEl] = useState<HTMLDivElement | null>(null);
  const [wrapW, setWrapW] = useState(0);
  const [gridW, setGridW] = useState(0);
  useEffect(() => {
    if (!wrapEl) return;
    const ro = new ResizeObserver((entries) => {
      for (const en of entries) {
        if (en.target === wrapEl) setWrapW(en.contentRect.width);
        else setGridW(en.contentRect.width);
      }
    });
    ro.observe(wrapEl);
    if (gridEl) ro.observe(gridEl);
    return () => ro.disconnect();
  }, [wrapEl, gridEl]);
  const fitCols = (w: number, min: number) => Math.max(1, Math.floor((w + 14) / (min + 14)));
  const cols = gridW > 0 ? Math.min(fitCols(wrapW - 56, 230), fitCols(gridW, 130)) : 4;
  const columns = useMemo(() => {
    const out: Item[][] = Array.from({ length: cols }, () => []);
    const heights = new Array(cols).fill(0);
    for (const it of shown) {
      const i = heights.indexOf(Math.min(...heights));
      out[i].push(it);
      heights[i] += it.height / Math.max(1, it.width) + 0.15;
    }
    return out;
  }, [shown, cols]);

  // A card that should have focus gets it back when a new layout remounted it.
  useEffect(() => {
    const id = wantFocus.current;
    if (id && (!document.activeElement || document.activeElement === document.body)) cardEl(id)?.focus({ preventScroll: true });
    const s = scrollReq.current;
    if (s && mode === 'canvas') {
      // The canvas pans with revealReq instead.
      scrollReq.current = null;
    } else if (s && cardEl(s.id)) {
      scrollReq.current = null;
      cardEl(s.id)!.scrollIntoView({ block: s.block });
    }
  }, [columns, mode, selected]);

  // A link from Insight: select that reference once the items are in.
  const focusDone = useRef<string | null>(null);
  useEffect(() => {
    if (!focus || !loaded || focusDone.current === focus) return;
    focusDone.current = focus;
    const it = items.find((i) => i.id.startsWith(focus));
    if (!it) {
      toast.info(`No image on this board has the ID ${focus}.`);
      return;
    }
    if (mode === 'canvas') setRevealReq({ id: it.id });
    else scrollReq.current = { id: it.id, block: 'center' };
    select(it.id);
  }, [focus, loaded, items, mode, select, toast]);

  // Keys of the board: Escape closes the inspector, arrows go to the next card, ⌘F searches.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'f') {
        const el = searchRef.current;
        if (!el) return;
        e.preventDefault();
        el.focus();
        el.select();
        return;
      }
      if (e.metaKey || e.ctrlKey || e.altKey || isField(e.target)) return;
      // Keys pressed outside the board (the sidebar, the language header, a message) are not for it.
      const t = e.target;
      if (t instanceof Node && t !== document.body && !wrapEl?.contains(t)) return;
      if (e.key === 'Escape') {
        if (!selected) return;
        e.preventDefault();
        closeInspector();
        return;
      }
      if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
      const focused = (document.activeElement as HTMLElement | null)?.closest<HTMLElement>('[data-card]')?.dataset.card ?? null;
      const cur = selected ?? focused;
      if (!cur) return;
      let next: string | undefined;
      if (mode === 'grid' && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
        const ci = columns.findIndex((c) => c.some((x) => x.id === cur));
        if (ci < 0) return;
        const ri = columns[ci].findIndex((x) => x.id === cur);
        next = columns[ci][ri + (e.key === 'ArrowDown' ? 1 : -1)]?.id;
      } else {
        const i = shown.findIndex((x) => x.id === cur);
        if (i < 0) return;
        next = shown[i + (e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : -1)]?.id;
      }
      if (!next) return;
      e.preventDefault();
      // Canvas: pan to the card. A focused card pans only on a visible focus, and the inspector can keep the focus.
      if (mode === 'canvas') setRevealReq({ id: next });
      if (!selected) {
        // Nothing open: the arrows only move the focus.
        focusCard(next);
        return;
      }
      // With focus in the inspector it stays there; otherwise it follows the card.
      const inInspector = !!document.activeElement?.closest('.inspector');
      select(next, inInspector);
      if (!inInspector) focusCard(next);
      else if (mode === 'grid') scrollReq.current = { id: next, block: 'nearest' };
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [shown, columns, selected, mode, wrapEl, closeInspector, focusCard, select]);

  const onCardKey = (e: ReactKeyboardEvent<HTMLElement>, id: string) => {
    if (e.target !== e.currentTarget || e.repeat) return;
    if (e.key !== 'Enter' && e.key !== ' ') return;
    e.preventDefault();
    if (selected === id) closeInspector();
    else select(id, true);
  };

  return (
    <div
      ref={setWrapEl}
      className={`board-wrap ${selectedItem ? 'with-inspector' : ''}`}
      onPointerDown={() => (wantFocus.current = null)}
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
      <div className={`board-main ${mode === 'canvas' && items.length > 0 ? 'canvas-mode' : ''}`}>
        <div className="toolbar">
          <button className="btn small primary" onClick={addFromDialog}>
            <ImagePlus size={13} /> Add images…
          </button>
          <button className="btn small" onClick={() => setBoardUrl(boardUrl === null ? '' : null)} title="Import the images of an Are.na channel or a Pinterest board">
            <Import size={13} /> Import board…
          </button>
          <span className="hint">Drag images, videos or 3D models (STL, 3MF, OBJ, STEP) · Paste a screenshot with ⌘V · Links and documents go to Sources</span>
          {busy > 0 && (
            <span className="chip chip-busy">
              <Loader size={12} className="spin" /> Importing…
            </span>
          )}
          <div className="toolbar-row">
            <label className="board-search" title="Search tags, notes, marks and what the analysis saw (⌘F)">
              <Search size={13} />
              <input
                ref={searchRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search this board"
                aria-label="Search this board"
              />
              {query && (
                <button className="icon-btn" onClick={() => setQuery('')} title="Clear the search" aria-label="Clear the search">
                  <X size={12} />
                </button>
              )}
            </label>
            <select value={filter} onChange={(e) => setFilter(e.target.value)} className="select small" aria-label="Tag">
              <option value="">All tags</option>
              {allTags.map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
            <label className="check small">
              <input type="checkbox" checked={showAnti} onChange={(e) => setShowAnti(e.target.checked)} />
              Show "not this"
            </label>
            <div className="seg" role="group" aria-label="Board view">
              <button className={`seg-btn ${mode === 'grid' ? 'active' : ''}`} onClick={() => setMode('grid')} aria-pressed={mode === 'grid'} title="Grid">
                <Grid2x2 size={13} /> Grid
              </button>
              <button className={`seg-btn ${mode === 'canvas' ? 'active' : ''}`} onClick={() => setMode('canvas')} aria-pressed={mode === 'canvas'} title="Canvas: arrange freely">
                <Move size={13} /> Canvas
              </button>
            </div>
          </div>
        </div>

        {boardUrl !== null && (
          <form className="board-import" onSubmit={importBoard}>
            <input
              autoFocus
              value={boardUrl}
              onChange={(e) => setBoardUrl(e.target.value)}
              placeholder="Link of an Are.na channel or a Pinterest board, then Return"
              disabled={!!boardBusy}
              onKeyDown={(e) => e.key === 'Escape' && !e.defaultPrevented && !boardBusy && setBoardUrl(null)}
            />
            <button className="btn small primary" type="submit" disabled={!!boardBusy || !boardUrl.trim()}>
              {boardBusy ? (
                <>
                  <Loader size={12} className="spin" /> {boardBusy}
                </>
              ) : (
                'Import'
              )}
            </button>
            <button className="btn small" type="button" onClick={() => setBoardUrl(null)} disabled={!!boardBusy}>
              Cancel
            </button>
            <span className="hint">Are.na: every image of a public channel. Pinterest: the newest 25 pins of a public board.</span>
          </form>
        )}

        {items.length > 0 && (palette.length > 0 || hits) && (
          <div className="board-strip">
            {palette.length > 0 && (
              <div className="palette" title="The main colours of the references, by weight. Click a colour to copy it.">
                {palette.map((c) => (
                  <button
                    key={c.hex}
                    className="palette-chip"
                    style={{ background: c.hex, flexGrow: Math.max(0.06, c.share) }}
                    title={`${c.hex} · ${Math.round(c.share * 100)} %`}
                    aria-label={`Copy ${c.hex}`}
                    onClick={() =>
                      navigator.clipboard
                        .writeText(c.hex)
                        .then(() => toast.info(`Copied ${c.hex}.`))
                        .catch(() => {})
                    }
                  />
                ))}
              </div>
            )}
            {hits && (
              <span className="hint">
                {shown.length === 0 ? `Nothing on this board matches “${query.trim()}”.` : `${shown.length} of ${items.length} match “${query.trim()}”, best first`}
              </span>
            )}
          </div>
        )}

        {items.length === 0 ? (
          <div className="board-empty">
            <div className="drop-target">
              <ImagePlus size={34} strokeWidth={1.3} className="drop-glyph" />
              <div className="drop-big">Drop images here</div>
              <p>
                Drag images from Finder or a browser, or paste a screenshot with ⌘V. A 3D model (STL, 3MF, OBJ, STEP) becomes four drawn
                views. The model analyzes each image. The language document appears after the analysis finishes.
              </p>
              <button className="btn primary" onClick={addFromDialog}>
                <ImagePlus size={14} /> Add images…
              </button>
            </div>
          </div>
        ) : mode === 'canvas' ? (
          <CanvasView
            languageId={language.id}
            items={shown}
            selected={selected}
            onSelect={select}
            renderCaption={renderCaption}
            cardLabel={cardLabel}
            onPlaced={onPlaced}
            revealReq={revealReq}
          />
        ) : (
          <div className="masonry" ref={setGridEl}>
            {columns.map((col, ci) => (
              <div className="mcol" key={ci}>
                {col.map((item) => (
              <figure
                key={item.id}
                data-card={item.id}
                tabIndex={0}
                role="button"
                aria-pressed={selected === item.id}
                aria-label={cardLabel(item)}
                className={`card ${item.anti ? 'anti' : ''} ${selected === item.id ? 'selected' : ''}`}
                onClick={() => select(item.id === selected ? null : item.id)}
                onKeyDown={(e) => onCardKey(e, item.id)}
                title={hitOf.get(item.id)?.snippets.join('\n')}
              >
                <img
                  src={fileUrl(item.thumb_rel)}
                  alt={item.original_name}
                  loading="lazy"
                  draggable={false}
                  style={{ aspectRatio: `${Math.max(1, item.width)} / ${Math.max(1, item.height)}` }}
                />
                {item.anti && <span className="anti-badge">Not this</span>}
                {item.kind === 'model' && <span className="model-badge">3D</span>}
                <figcaption>
                  <Caption item={item} />
                </figcaption>
              </figure>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>

      {selectedItem && (
        <ItemInspector
          key={selectedItem.id}
          item={selectedItem}
          tick={tick}
          focusOnOpen={openedByKey.current}
          onClose={closeInspector}
          onChanged={() => {
            load();
            onChanged();
          }}
          onRemoved={() => {
            const removedId = selectedItem.id;
            load();
            onChanged();
            // The removal takes a moment: when another card was opened meanwhile, it stays open.
            if (selectedRef.current !== removedId) return;
            // Focus goes to the next card (or the one before it).
            const i = shown.findIndex((x) => x.id === removedId);
            const next = i < 0 ? undefined : (shown[i + 1] ?? shown[i - 1]);
            const back = focusWasInInspector();
            setSelected((cur) => (cur === removedId ? null : cur));
            if (next && back) focusCard(next.id);
          }}
        />
      )}

      {dragging && (
        <div className="drop-overlay">
          <div>Drop to add to {language.name}</div>
        </div>
      )}
    </div>
  );
}
