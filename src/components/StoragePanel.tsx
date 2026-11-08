import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { listen } from '@tauri-apps/api/event';
import { ArrowRight, FolderInput, FolderOpen, FolderSearch, HardDrive, Loader, TriangleAlert } from 'lucide-react';
import { api, AppInfo, errText, FolderInfo, fmtBytes, MoveProgress } from '../api';
import { useToast } from './Toast';

/** The folder that receives a moved library: the chosen folder if it is empty, else a "Corydoras" folder inside it. */
async function moveTarget(picked: string): Promise<FolderInfo> {
  const info = await api.folderInfo(picked);
  if (info.empty && !info.is_library) return info;
  const sep = picked.endsWith('/') ? '' : '/';
  for (const name of ['Corydoras', 'Corydoras Library', 'Corydoras 2', 'Corydoras 3']) {
    const inner = await api.folderInfo(picked + sep + name);
    if (inner.empty && !inner.is_library) return inner;
  }
  return info;
}

export default function StoragePanel({ info, onInfo, onChanged }: { info: AppInfo; onInfo: (i: AppInfo) => void; onChanged: () => void }) {
  const toast = useToast();
  const [target, setTarget] = useState<FolderInfo | null>(null);
  const [trashOld, setTrashOld] = useState(false);
  const [moving, setMoving] = useState(false);
  const [progress, setProgress] = useState<MoveProgress | null>(null);
  const [newLib, setNewLib] = useState<FolderInfo | null>(null);

  useEffect(() => {
    const un = listen<MoveProgress>('library-move-progress', (e) => setProgress(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  const chooseMove = async () => {
    const picked = await open({ directory: true, multiple: false, canCreateDirectories: true, title: 'Choose where the library goes' });
    if (typeof picked !== 'string') return;
    try {
      setTarget(await moveTarget(picked));
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const doMove = async () => {
    if (!target) return;
    setMoving(true);
    setProgress(null);
    try {
      const r = await api.moveLibrary(target.path, trashOld);
      onInfo(r.info);
      setTarget(null);
      toast.info(
        `Moved ${r.report.files} files (${fmtBytes(r.report.bytes)}) to ${r.report.new_path}.` +
          (r.trashed ? ' The old folder is in the Trash.' : r.trash_error ? ` ${r.trash_error}` : ` The old folder stays at ${r.old_path}.`),
      );
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setMoving(false);
    }
  };

  const switchTo = async (path: string) => {
    try {
      onInfo(await api.setLibrary(path));
      setNewLib(null);
      toast.info(`Opened the library at ${path}.`);
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const chooseOther = async () => {
    const picked = await open({ directory: true, multiple: false, canCreateDirectories: true, title: 'Choose a library folder, or an empty folder for a new library' });
    if (typeof picked !== 'string') return;
    try {
      const fi = await api.folderInfo(picked);
      if (fi.is_library) return switchTo(picked);
      if (fi.empty) return setNewLib(fi);
      toast.error('That folder has other files in it. Choose a library folder, or an empty folder for a new library.');
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const pct = progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0;

  return (
    <section className="panel">
      <h3>
        <HardDrive size={15} /> Library location
      </h3>
      <p className="muted small">
        The library folder holds the images, documents, analyses and versions of all languages. API keys stay in the app's own folder, and the provider
        settings stay with the app, so they do not change when you move or switch the library.
      </p>
      <div className="lib-path">
        <FolderOpen size={15} />
        <span className="mono small wrap">{info.library_path}</span>
        {info.library_path && (
          <button className="btn small" onClick={() => api.revealPath(info.library_path!).catch((e) => toast.error(errText(e)))}>
            <FolderSearch size={13} /> Show in Finder
          </button>
        )}
      </div>
      {info.library_warning && (
        <div className="warn-box">
          <TriangleAlert size={14} /> {info.library_warning}
        </div>
      )}

      <div className="row">
        <button className="btn small primary" onClick={chooseMove} disabled={moving}>
          <FolderInput size={13} /> Move library…
        </button>
        <button className="btn small" onClick={chooseOther} disabled={moving}>
          <FolderOpen size={13} /> Open another library…
        </button>
      </div>

      {target && (
        <div className="move-card">
          <div className="move-paths">
            <span className="mono small wrap">{info.library_path}</span>
            <ArrowRight size={14} />
            <span className="mono small wrap">{target.path}</span>
          </div>
          {target.is_library || !target.empty ? (
            <div className="error-box">That folder has files in it. Choose an empty folder, or a new folder.</div>
          ) : (
            <>
              {target.warning && (
                <div className="warn-box">
                  <TriangleAlert size={14} /> {target.warning}
                </div>
              )}
              <ul className="small move-steps">
                <li>The queue stops. A running job stops and runs again after the move.</li>
                <li>Corydoras copies the library, checks the copy, and then uses it.</li>
                <li>If an agent starts the MCP server with “--library” and the old path, change that path. Without “--library”, the server follows the app.</li>
              </ul>
              <label className="check small">
                <input type="checkbox" checked={trashOld} onChange={(e) => setTrashOld(e.target.checked)} />
                Move the old folder to the Trash after the copy passes its check
              </label>
              {moving && (
                <div className="move-progress">
                  <div className="bar">
                    <div style={{ width: `${pct}%` }} />
                  </div>
                  <span className="small muted">
                    <Loader size={12} className="spin" /> {progress ? `${fmtBytes(progress.done)} of ${fmtBytes(progress.total)}` : 'Stopping the queue…'}
                  </span>
                </div>
              )}
              <div className="row">
                <button className="btn small primary" onClick={doMove} disabled={moving}>
                  <FolderInput size={13} /> {moving ? 'Moving…' : 'Move library'}
                </button>
                <button className="btn small" onClick={() => setTarget(null)} disabled={moving}>
                  Cancel
                </button>
              </div>
            </>
          )}
        </div>
      )}

      {newLib && (
        <div className="move-card">
          <p className="small">
            <span className="mono">{newLib.path}</span> is empty. Start a new, empty library there? The current library stays as it is, and you can
            come back to it from this list.
          </p>
          {newLib.warning && (
            <div className="warn-box">
              <TriangleAlert size={14} /> {newLib.warning}
            </div>
          )}
          <div className="row">
            <button className="btn small primary" onClick={() => switchTo(newLib.path)}>
              Start a new library
            </button>
            <button className="btn small" onClick={() => setNewLib(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {info.recent_libraries.length > 0 && (
        <div className="recent-libs">
          <div className="eyebrow">Other libraries</div>
          {info.recent_libraries.map((p) => (
            <div key={p} className="recent-row">
              <span className="mono small wrap">{p}</span>
              <button className="btn small" onClick={() => switchTo(p)} disabled={moving}>
                Open
              </button>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
