import { useEffect, useRef, useState } from 'react';
import { ImageIcon } from 'lucide-react';
import { inventoryApi } from '../inventoryApi';

/** One IPC request per visible picture. The API shares each result across rows. */
export default function WorkshopThumbnail({ photoKey }: { photoKey: string | null }) {
  const container = useRef<HTMLSpanElement>(null);
  const [src, setSrc] = useState('');
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    const node = container.current;
    if (!node || !photoKey) return;
    let active = true;
    const load = () => {
      inventoryApi.photo(photoKey, true).then(url => {
        if (active) setSrc(url);
      }).catch(() => { if (active) setFailed(true); });
    };
    let observer: IntersectionObserver | undefined;
    if (typeof IntersectionObserver === 'undefined') load();
    else {
      observer = new IntersectionObserver(entries => {
        if (!entries.some(entry => entry.isIntersecting)) return;
        observer?.disconnect();
        load();
      }, { rootMargin: '200px 0px' });
      observer.observe(node);
    }
    return () => { active = false; observer?.disconnect(); };
  }, [photoKey]);

  return <span className="workshop-thumb" ref={container} aria-hidden="true"
    title={failed ? 'Photo unavailable' : !photoKey ? 'No photo' : undefined}>
    {src && !failed
      ? <img src={src} data-workshop-thumbnail="" alt="" draggable={false} decoding="async" onError={() => setFailed(true)} />
      : <ImageIcon size={18} strokeWidth={1.4} />}
  </span>;
}
