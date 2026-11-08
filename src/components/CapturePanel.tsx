import { useCallback, useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Camera } from 'lucide-react';
import { api, CaptureView, errText, Language, shortcutFromEvent, shortcutLabel } from '../api';
import { useToast } from './Toast';

const DEFAULT_SHORTCUT = 'Control+Alt+Super+KeyC';

export default function CapturePanel() {
  const toast = useToast();
  const [view, setView] = useState<CaptureView | null>(null);
  const [languages, setLanguages] = useState<Language[]>([]);
  const [recording, setRecording] = useState(false);

  const load = useCallback(() => {
    api.getCapture().then(setView).catch((e) => toast.error(errText(e)));
    api.listLanguages().then(setLanguages).catch(() => {});
  }, [toast]);

  useEffect(load, [load]);

  // The menu bar can change the capture language too.
  useEffect(() => {
    const un = listen('capture-changed', load);
    return () => {
      un.then((f) => f());
    };
  }, [load]);

  const save = useCallback(
    async (languageId: string | null, shortcut: string | null) => {
      try {
        setView(await api.setCapture(languageId, shortcut));
        return true;
      } catch (e) {
        toast.error(errText(e));
        return false;
      }
    },
    [toast],
  );

  // Record a shortcut: the next key press with a modifier.
  useEffect(() => {
    if (!recording || !view) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === 'Escape' && !e.metaKey && !e.ctrlKey && !e.altKey) {
        setRecording(false);
        return;
      }
      const s = shortcutFromEvent(e);
      if (!s) return;
      setRecording(false);
      save(view.language_id, s).then((ok) => ok && toast.info(`Capture shortcut: ${shortcutLabel(s)}`));
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [recording, view, save, toast]);

  if (!view) return null;

  return (
    <section className="panel">
      <h3>
        <Camera size={15} /> Screen capture
      </h3>
      <p className="muted small">
        The fish in the menu bar and the keyboard shortcut capture part of the screen into a language, also when the Corydoras window is
        closed. Drag over a region, press Space to pick a window, or press Escape to cancel.
      </p>
      <div className="grid2">
        <label className="field">
          <span>Capture into</span>
          <select
            className="select"
            value={view.language_id ?? ''}
            onChange={(e) => save(e.target.value || null, view.shortcut)}
            disabled={languages.length === 0}
          >
            {languages.length === 0 && <option value="">Make a language first</option>}
            {languages.map((l) => (
              <option key={l.id} value={l.id}>
                {l.name}
              </option>
            ))}
          </select>
        </label>
        <div className="field">
          <span>Keyboard shortcut</span>
          <div className="row shortcut-row">
            <button
              className={`btn small shortcut-key ${recording ? 'recording' : ''}`}
              onClick={() => setRecording((r) => !r)}
              aria-label={recording ? 'Press the new shortcut' : 'Change the shortcut'}
            >
              {recording ? 'Press keys…' : view.shortcut ? shortcutLabel(view.shortcut) : 'None'}
            </button>
            {view.shortcut && !recording && (
              <button className="link-btn" onClick={() => save(view.language_id, null)}>
                Clear
              </button>
            )}
            {view.shortcut !== DEFAULT_SHORTCUT && !recording && (
              <button className="link-btn" onClick={() => save(view.language_id, DEFAULT_SHORTCUT)}>
                Use {shortcutLabel(DEFAULT_SHORTCUT)}
              </button>
            )}
          </div>
        </div>
      </div>
      {recording && (
        <p className="muted small">Press the keys with at least one of ⌃ ⌥ ⌘ (or a function key). Escape stops.</p>
      )}
      {view.shortcut_error && <div className="error-box">{view.shortcut_error}</div>}
      <p className="muted small">
        The first capture asks for Screen Recording access (System Settings › Privacy &amp; Security). This build is not signed with a
        fixed identity, so macOS can ask again after an update.
      </p>
      <div className="row">
        <button className="btn small" onClick={() => api.captureNow()} disabled={!view.language_id}>
          <Camera size={13} /> Capture now
        </button>
      </div>
    </section>
  );
}
