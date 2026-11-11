import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, AppInfo, errText } from '../api';
import { FolderOpen } from 'lucide-react';
import Logo from './Logo';
import { useToast } from './Toast';

export default function Onboarding({ info, onDone }: { info: AppInfo; onDone: (i: AppInfo) => void }) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);

  const use = async (path: string) => {
    setBusy(true);
    try {
      onDone(await api.setLibrary(path));
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const choose = async () => {
    const picked = await open({ directory: true, multiple: false, title: 'Choose the Corydoras library folder' });
    if (typeof picked === 'string') await use(picked);
  };

  return (
    <div className="onboarding" data-tauri-drag-region>
      <div className="onboarding-card">
        <div className="brand big">
          <Logo size={76} />
          <span className="brand-word">Corydoras</span>
        </div>
        <h1>Mood boards that become design languages.</h1>
        <p>
          Corydoras keeps your reference images, notes and language documents in one library folder. Plain files hold the images
          and documents, and SQLite holds the index. You can back up or sync the folder with any tool.
        </p>
        <p>Next, add a model provider in Settings.</p>
        <div className="onboarding-actions">
          <button className="btn primary" disabled={busy} onClick={() => use(info.default_library)}>
            Use {info.default_library.replace(/^\/Users\/[^/]+/, '~')}
          </button>
          <button className="btn" disabled={busy} onClick={choose}>
            <FolderOpen size={14} /> Choose another folder…
          </button>
        </div>
      </div>
    </div>
  );
}
