import { DragEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { Grid2x2, ImagePlus, Loader, Move } from 'lucide-react';
import { api, CanvasPlacement, DOC_EXT_RE, errText, fileToBase64, fileUrl, fmtDuration, IMAGE_URL_RE, Item, Language, VIDEO_EXT_RE, VIDEO_EXTENSIONS, VideoNote } from '../api';
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

function Caption({ item }: { item: Item }) {
  return (
    <>
      <StatusDot item={item} />
      <span className="weight" title={`Weight ${item.weight}/5`}>
        {'●'.repeat(item.weight)}
        <span className="weight-off">{'●'.repeat(5 - item.weight)}</span>
      </span>
      {item.region_count > 0 && <span className="cap-meta">{item.region_count} mark{item.region_count === 1 ? '' : 's'}</span>}
      <span className="cap-tags">{item.tags.slice(0, 3).join(' · ')}</span>
    </>
  );
}

const renderCaption = (item: Item) => <Caption item={item} />;

function StatusDot({ item }: { item: Item }) {
  const label =
    item.analysis_status === 'done'
      ? 'Analyzed'
      : item.analysis_status === 'error'
        ? `Analysis failed: ${item.analysis_error ?? ''}`
        : item.analysis_status === 'analyzing'
          ? 'Analyzing'
          : 'In the queue';
  return <span className={`dot status-${item.analysis_status}`} title={label} />;
}

export default function Board({
  language,
  tick,
  onChanged,
  onSources,
}: {
  language: Language;
  tick: number;
  onChanged: () => void;
  onSources: () => void;
}) {
  const toast = useToast();
  const [items, setItems] = useState<Item[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState(0);
  const [filter, setFilter] = useState('');
  const [showAnti, setShowAnti] = useState(true);
  const [mode, setModeState] = useState<BoardMode>(() => loadMode(language.id));
  const setMode = (m: BoardMode) => {
    setModeState(m);
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
      .then(setItems)
      .catch((e) => toast.error(errText(e)));
  }, [language.id, toast]);

  useEffect(load, [load, tick]);

  const afterImport = useCallback(
    (added: number, errors: string[], videos: VideoNote[] = []) => {
      errors.forEach((e) => toast.error(e));
      for (const v of videos) {
        toast.info(
          v.frames > 0
            ? `${v.frames} key frame${v.frames === 1 ? '' : 's'} from “${v.name}” (${fmtDuration(v.duration * 1000)}) added. The analysis starts now.`
            : `No usable frames in “${v.name}”.`,
        );
      }
      const stills = added - videos.reduce((n, v) => n + v.frames, 0);
      if (stills > 0) toast.info(`${stills} image${stills === 1 ? '' : 's'} added. The analysis starts now.`);
      load();
      onChanged();
    },
    [load, onChanged, toast],
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
      title: 'Add images or videos',
      filters: [
        { name: 'Images and videos', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'tif', 'tiff', ...VIDEO_EXTENSIONS] },
      ],
    });
    const paths = Array.isArray(picked) ? picked : typeof picked === 'string' ? [picked] : [];
    if (paths.length === 0) return;
    if (paths.some((p) => VIDEO_EXT_RE.test(p))) toast.info('Reading key frames from the video…');
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

  const shown = useMemo(
    () => items.filter((i) => (showAnti || !i.anti) && (!filter || i.tags.includes(filter))),
    [items, showAnti, filter],
  );
  const selectedItem = items.find((i) => i.id === selected) ?? null;

  // The canvas saved new places: keep the item list in step, so a switch to Grid
  // and back (or any re-render) starts from the saved places.
  const onPlaced = useCallback((places: CanvasPlacement[]) => {
    const byId = new Map(places.map((p) => [p.id, { x: p.x, y: p.y, w: p.w }]));
    setItems((list) => list.map((i) => (byId.has(i.id) ? { ...i, canvas: byId.get(i.id)! } : i)));
  }, []);

  // Masonry: put each card in the shortest column. The image sizes are known, so the layout is stable.
  const [gridEl, setGridEl] = useState<HTMLDivElement | null>(null);
  const [cols, setCols] = useState(4);
  useEffect(() => {
    if (!gridEl) return;
    const ro = new ResizeObserver(([entry]) => {
      setCols(Math.max(1, Math.floor((entry.contentRect.width + 14) / (230 + 14))));
    });
    ro.observe(gridEl);
    return () => ro.disconnect();
  }, [gridEl]);
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

  return (
    <div
      className={`board-wrap ${selectedItem ? 'with-inspector' : ''}`}
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
          <span className="hint">Drag from Finder or a browser · Videos give their key frames · Paste a screenshot with ⌘V · Links and documents go to Sources</span>
          <div className="spacer" />
          {busy > 0 && (
            <span className="chip chip-busy">
              <Loader size={12} className="spin" /> Importing…
            </span>
          )}
          <select value={filter} onChange={(e) => setFilter(e.target.value)} className="select small">
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

        {items.length === 0 ? (
          <div className="board-empty">
            <div className="drop-target">
              <ImagePlus size={34} strokeWidth={1.3} className="drop-glyph" />
              <div className="drop-big">Drop images here</div>
              <p>
                Drag images from Finder or a browser, or paste a screenshot with ⌘V. Each image gets analyzed automatically. The language
                document appears after the analysis finishes.
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
            onSelect={setSelected}
            renderCaption={renderCaption}
            onPlaced={onPlaced}
          />
        ) : (
          <div className="masonry" ref={setGridEl}>
            {columns.map((col, ci) => (
              <div className="mcol" key={ci}>
                {col.map((item) => (
              <figure
                key={item.id}
                className={`card ${item.anti ? 'anti' : ''} ${selected === item.id ? 'selected' : ''}`}
                onClick={() => setSelected(item.id === selected ? null : item.id)}
              >
                <img
                  src={fileUrl(item.thumb_rel)}
                  alt={item.original_name}
                  loading="lazy"
                  draggable={false}
                  style={{ aspectRatio: `${Math.max(1, item.width)} / ${Math.max(1, item.height)}` }}
                />
                {item.anti && <span className="anti-badge">Not this</span>}
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
          onClose={() => setSelected(null)}
          onChanged={() => {
            load();
            onChanged();
          }}
          onRemoved={() => {
            setSelected(null);
            load();
            onChanged();
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
