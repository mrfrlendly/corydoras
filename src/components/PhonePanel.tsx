import { useEffect, useState } from 'react';
import { Smartphone } from 'lucide-react';
import { api, errText, type PhoneView } from '../api';
import { useToast } from './Toast';
import { useConfirmRow } from './useConfirmRow';

/** The phone link: a page on the Wi-Fi where a phone photographs labels and saves inventory items.
 *  Shown while the inventory module is on, and always while the link runs, so it can be turned off. */
export default function PhonePanel({ inventoryEnabled }: { inventoryEnabled: boolean }) {
  const toast = useToast();
  const [view, setView] = useState<PhoneView | null>(null);
  const [port, setPort] = useState(47822);
  const [confirmToken, setConfirmToken] = useState(false);
  const tokenTrigger = useConfirmRow(confirmToken, () => setConfirmToken(false));

  const refresh = () =>
    api
      .getPhone()
      .then((v) => {
        setView(v);
        setPort(v.port);
      })
      .catch((e) => toast.error(errText(e)));

  // Once, when the panel opens; Refresh asks again.
  useEffect(() => void refresh(), []); // eslint-disable-line react-hooks/exhaustive-deps

  const apply = async (enabled: boolean, newToken = false) => {
    try {
      const v = await api.setPhone(enabled, port, newToken);
      setView(v);
      setPort(v.port);
      setConfirmToken(false);
      if (newToken) toast.info('New code made. Scan it again on your phone.');
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const copy = (text: string) =>
    navigator.clipboard
      .writeText(text)
      .then(() => toast.info('Copied the phone link. It contains the token: send it only to your own phone.'))
      .catch(() => toast.error('Copy failed.'));

  if (!view || (!view.enabled && !inventoryEnabled)) return null;
  return (
    <section className="panel" id="settings-phone" tabIndex={-1}>
      <h3>
        <Smartphone size={15} /> Phone
      </h3>
      <p className="muted small">
        Photograph labels on your phone and save the items to this library. Your phone opens a page that this app serves on your Wi-Fi; the Mac must be awake
        with Corydoras open.
      </p>
      <label className="check">
        <input type="checkbox" checked={view.enabled} onChange={(e) => apply(e.target.checked)} />
        Let my phone add inventory items over Wi-Fi
      </label>
      {view.error && <div className="error-box">{view.error}</div>}
      {view.enabled && !inventoryEnabled && (
        <p className="small">Workshop inventory is off for this library, so the phone can neither read nor save. Turn the module on, or turn the phone link off here.</p>
      )}
      {view.enabled && (
        <>
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
          {view.running && !view.url && <p className="small">This Mac has no Wi-Fi address right now.</p>}
          {view.url && (
            <div className="phone-link">
              {/* The SVG comes from the app (a QR code of the link), not from user text. */}
              {view.qr_svg && <div className="phone-qr" aria-label="QR code for the phone link" role="img" dangerouslySetInnerHTML={{ __html: view.qr_svg }} />}
              <div className="phone-link-text">
                <p className="small">Scan the code with your phone’s camera. The phone stays connected until you make a new code.</p>
                <div className="row nowrap" style={{ marginTop: 0 }}>
                  <input className="mono grow-input" readOnly value={view.url} onFocus={(e) => e.target.select()} aria-label="Phone link" />
                  <button className="btn small" onClick={() => copy(view.url!)}>
                    Copy
                  </button>
                </div>
                {view.local_url && (
                  <>
                    <p className="muted small">
                      Or by this Mac’s name, which keeps working when the Wi-Fi address changes. Some phones cannot open .local addresses.
                    </p>
                    <div className="row nowrap" style={{ marginTop: 0 }}>
                      <input className="mono grow-input" readOnly value={view.local_url} onFocus={(e) => e.target.select()} aria-label="Phone link by name" />
                      <button className="btn small" onClick={() => copy(view.local_url!)}>
                        Copy
                      </button>
                    </div>
                  </>
                )}
                <div className="row" style={{ marginTop: 0 }}>
                  <button className="btn small" onClick={() => void refresh()} title="Look up this Mac’s Wi-Fi address again">
                    Refresh
                  </button>
                  {!confirmToken && (
                    <button ref={tokenTrigger} className="btn small" onClick={() => setConfirmToken(true)}>
                      New token…
                    </button>
                  )}
                </div>
              </div>
            </div>
          )}
          {confirmToken && (
            <div className="row confirm-row">
              <button className="btn small danger" onClick={() => apply(true, true)}>
                Make a new token
              </button>
              <button className="btn small" autoFocus onClick={() => setConfirmToken(false)}>
                Keep
              </button>
              <span className="muted small">Phones that scanned the old code are disconnected.</span>
            </div>
          )}
          <p className="muted small">
            While this is on, any device on the same network as this Mac can open the page, wherever the Mac is. Only a phone that scanned this code can read labels
            or save. The connection is not encrypted, so use it on a network you trust. Each read uses your analysis provider and counts toward the monthly budget,
            like a read on the Mac.
          </p>
        </>
      )}
    </section>
  );
}
