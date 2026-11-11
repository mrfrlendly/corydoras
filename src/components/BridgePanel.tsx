import { useEffect, useState } from 'react';
import { Puzzle } from 'lucide-react';
import { api, BridgeView, errText } from '../api';
import { useToast } from './Toast';
import { useConfirmRow } from './useConfirmRow';

/** The local bridge that the browser extension sends images and pages to. */
export default function BridgePanel() {
  const toast = useToast();
  const [view, setView] = useState<BridgeView | null>(null);
  const [port, setPort] = useState(47821);
  const [confirmToken, setConfirmToken] = useState(false);
  const tokenTrigger = useConfirmRow(confirmToken, () => setConfirmToken(false));

  useEffect(() => {
    api
      .getBridge()
      .then((v) => {
        setView(v);
        setPort(v.port);
      })
      .catch((e) => toast.error(errText(e)));
  }, [toast]);

  const apply = async (enabled: boolean, newToken = false) => {
    try {
      const v = await api.setBridge(enabled, port, newToken);
      setView(v);
      setPort(v.port);
      setConfirmToken(false);
      if (newToken) toast.info('New token made. Paste it in the options of the extension.');
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const copy = (text: string, what: string) =>
    navigator.clipboard
      .writeText(text)
      .then(() => toast.info(`Copied the ${what}.`))
      .catch(() => toast.error('Copy failed.'));

  if (!view) return null;
  return (
    <section className="panel" id="settings-extension" tabIndex={-1}>
      <h3>
        <Puzzle size={15} /> Browser extension
      </h3>
      <p className="muted small">
        The Corydoras extension adds a right-click command to Chrome, Edge, Brave and Arc: “Add image to Corydoras” and “Add page to Corydoras sources”.
        It talks to this app through a local bridge. The bridge listens on this Mac only, and it answers only requests that carry the token below.
      </p>
      <label className="check">
        <input type="checkbox" checked={view.enabled} onChange={(e) => apply(e.target.checked)} />
        Let the browser extension add references
      </label>
      {view.error && <div className="error-box">{view.error}</div>}
      {view.enabled && (
        <>
          <div className="grid2">
            <label className="field">
              <span>
                Port {view.running ? <span className="chip chip-ok">Listening</span> : <span className="chip chip-warn">Not running</span>}
              </span>
              <div className="row nowrap" style={{ marginTop: 0 }}>
                <input type="number" min={1024} max={65535} value={port} onChange={(e) => setPort(Number(e.target.value))} />
                {port !== view.port && (
                  <button className="btn small primary" onClick={() => apply(true)}>
                    Use this port
                  </button>
                )}
              </div>
            </label>
            <div className="field">
              <span>Token</span>
              <div className="row nowrap" style={{ marginTop: 0 }}>
                <input className="mono grow-input" readOnly value={view.token} onFocus={(e) => e.target.select()} aria-label="Token" />
                <button className="btn small" onClick={() => copy(view.token, 'token')}>
                  Copy
                </button>
                {!confirmToken && (
                  <button ref={tokenTrigger} className="btn small" onClick={() => setConfirmToken(true)} title="The old token stops working">
                    New token…
                  </button>
                )}
              </div>
            </div>
          </div>
          {confirmToken && (
            <div className="row confirm-row">
              <button className="btn small danger" onClick={() => apply(true, true)}>
                Make a new token
              </button>
              <button className="btn small" autoFocus onClick={() => setConfirmToken(false)}>
                Keep
              </button>
              <span className="muted small">The extension stops working until you paste the new token.</span>
            </div>
          )}
          <ol className="small how-to">
            <li>
              In the browser, open <span className="mono">chrome://extensions</span>, turn on Developer mode, click <b>Load unpacked</b>, and choose the{' '}
              <span className="mono">extension</span> folder of the Corydoras project.
            </li>
            <li>Open the options of the extension, and paste the port and the token.</li>
            <li>Right-click an image or a page. Without a choice, it goes to the language that screen captures go to.</li>
          </ol>
        </>
      )}
    </section>
  );
}
