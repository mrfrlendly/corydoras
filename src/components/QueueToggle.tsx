import { api, errText, StatusReport } from '../api';
import { Play, Square } from 'lucide-react';
import { useToast } from './Toast';

/** Sidebar control: automatic or manual queue, plus Run / Stop in manual mode. */
export default function QueueToggle({ status, onChanged }: { status: StatusReport | null; onChanged: () => void }) {
  const toast = useToast();
  if (!status) return null;
  const run = (fn: () => Promise<void>) => fn().then(onChanged).catch((e) => toast.error(errText(e)));
  const waiting = status.queued - (status.current ? 1 : 0);

  return (
    <div className="queue-toggle">
      <span className="queue-label" id="queue-label">
        Queue
      </span>
      <div
        className="seg"
        role="group"
        aria-labelledby="queue-label"
        title="Automatic: jobs run as soon as they are due. Manual: jobs wait until you click Run queue."
      >
        <button className={!status.manual ? 'active' : ''} aria-pressed={!status.manual} onClick={() => status.manual && run(() => api.setManual(false))}>
          Auto
        </button>
        <button className={status.manual ? 'active' : ''} aria-pressed={status.manual} onClick={() => !status.manual && run(() => api.setManual(true))}>
          Manual
        </button>
      </div>
      {status.manual &&
        (status.running ? (
          <button className="btn small" onClick={() => run(() => api.runQueue(false))} title="The running job finishes; the rest wait">
            <span className="dot dot-busy" /> Running · <Square size={11} fill="currentColor" /> Stop
          </button>
        ) : (
          <button
            className="btn small primary"
            disabled={status.queued === 0}
            onClick={() => run(() => api.runQueue(true))}
            title="Run every waiting job, then wait again"
          >
            <Play size={12} fill="currentColor" /> Run queue{status.queued > 0 ? ` (${waiting > 0 ? waiting : status.queued})` : ''}
          </button>
        ))}
    </div>
  );
}
