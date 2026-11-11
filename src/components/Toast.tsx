import { createContext, ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { CircleAlert, CircleCheck } from 'lucide-react';

type Kind = 'info' | 'error';

/** An optional button in a toast. */
export interface ToastAction {
  label: string;
  run: () => void;
}

interface ToastMsg {
  id: number;
  kind: Kind;
  text: string;
  action?: ToastAction;
  /** Set while the exit transition runs; the toast is removed after it. */
  leaving?: boolean;
}

interface ToastApi {
  info: (text: string, action?: ToastAction) => void;
  error: (text: string) => void;
}

/** Length of the exit transition in styles.css (.toast-slot.leaving). */
const EXIT_MS = 160;

const Ctx = createContext<ToastApi>({ info: () => {}, error: () => {} });

function lifetime(m: ToastMsg) {
  // A toast with a button stays longer: you must read it and then reach the button.
  return m.kind === 'error' ? 8000 : m.action ? 10000 : 3500;
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [msgs, setMsgs] = useState<ToastMsg[]>([]);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [hidden, setHidden] = useState(() => document.hidden);

  // The timers stop while the pointer or the keyboard focus is in the toasts, and while the
  // window is hidden: a toast must not expire before anyone could read it.
  useEffect(() => {
    const onVis = () => setHidden(document.hidden);
    document.addEventListener('visibilitychange', onVis);
    return () => document.removeEventListener('visibilitychange', onVis);
  }, []);

  // A removed toast takes the focus with it without a blur event; drop the pause then.
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (focused && !box.current?.contains(document.activeElement)) setFocused(false);
  }, [msgs, focused]);

  const dismiss = useCallback((id: number) => {
    setMsgs((all) => all.map((x) => (x.id === id ? { ...x, leaving: true } : x)));
    setTimeout(() => setMsgs((all) => all.filter((x) => x.id !== id)), EXIT_MS);
  }, []);

  const push = useCallback((kind: Kind, text: string, action?: ToastAction) => {
    const id = Date.now() + Math.random();
    // At most four on screen; the oldest goes without a transition.
    setMsgs((m) => [...m.filter((x) => !x.leaving).slice(-3), { id, kind, text, action }]);
  }, []);

  const value = useMemo(
    () => ({ info: (t: string, a?: ToastAction) => push('info', t, a), error: (t: string) => push('error', t) }),
    [push],
  );

  return (
    <Ctx.Provider value={value}>
      {children}
      <div
        ref={box}
        className="toasts"
        aria-live="polite"
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        onFocus={() => setFocused(true)}
        onBlur={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(false);
        }}
      >
        {msgs.map((m) => (
          <ToastItem key={m.id} msg={m} paused={hovered || focused || hidden} onDismiss={dismiss} />
        ))}
      </div>
    </Ctx.Provider>
  );
}

function ToastItem({ msg: m, paused, onDismiss }: { msg: ToastMsg; paused: boolean; onDismiss: (id: number) => void }) {
  const remaining = useRef(lifetime(m));

  useEffect(() => {
    if (paused || m.leaving) return;
    const start = Date.now();
    const t = setTimeout(() => onDismiss(m.id), remaining.current);
    return () => {
      clearTimeout(t);
      remaining.current = Math.max(600, remaining.current - (Date.now() - start));
    };
  }, [paused, m.leaving, m.id, onDismiss]);

  return (
    <div className={`toast-slot ${m.leaving ? 'leaving' : ''}`}>
      <div className="toast-clip">
        <div className={`toast toast-${m.kind}`} onClick={() => onDismiss(m.id)} aria-live={m.kind === 'error' ? 'assertive' : undefined}>
          {m.kind === 'error' ? <CircleAlert size={15} className="toast-icon" /> : <CircleCheck size={15} className="toast-icon" />}
          <span className="toast-text">{m.text}</span>
          {m.action && (
            <button
              className="toast-action"
              onClick={(e) => {
                e.stopPropagation();
                m.action!.run();
                onDismiss(m.id);
              }}
            >
              {m.action.label}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

export function useToast(): ToastApi {
  return useContext(Ctx);
}
