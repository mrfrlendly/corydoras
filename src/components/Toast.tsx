import { createContext, ReactNode, useCallback, useContext, useMemo, useState } from 'react';
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
}

interface ToastApi {
  info: (text: string, action?: ToastAction) => void;
  error: (text: string) => void;
}

const Ctx = createContext<ToastApi>({ info: () => {}, error: () => {} });

export function ToastProvider({ children }: { children: ReactNode }) {
  const [msgs, setMsgs] = useState<ToastMsg[]>([]);
  const push = useCallback((kind: Kind, text: string, action?: ToastAction) => {
    const id = Date.now() + Math.random();
    setMsgs((m) => [...m.slice(-3), { id, kind, text, action }]);
    setTimeout(() => setMsgs((m) => m.filter((x) => x.id !== id)), kind === 'error' ? 8000 : action ? 6000 : 3500);
  }, []);
  const value = useMemo(
    () => ({ info: (t: string, a?: ToastAction) => push('info', t, a), error: (t: string) => push('error', t) }),
    [push],
  );
  const dismiss = (id: number) => setMsgs((all) => all.filter((x) => x.id !== id));
  return (
    <Ctx.Provider value={value}>
      {children}
      <div className="toasts">
        {msgs.map((m) => (
          <div key={m.id} className={`toast toast-${m.kind}`} onClick={() => dismiss(m.id)}>
            {m.kind === 'error' ? <CircleAlert size={15} className="toast-icon" /> : <CircleCheck size={15} className="toast-icon" />}
            <span className="toast-text">{m.text}</span>
            {m.action && (
              <button
                className="toast-action"
                onClick={(e) => {
                  e.stopPropagation();
                  m.action!.run();
                  dismiss(m.id);
                }}
              >
                {m.action.label}
              </button>
            )}
          </div>
        ))}
      </div>
    </Ctx.Provider>
  );
}

export function useToast(): ToastApi {
  return useContext(Ctx);
}
