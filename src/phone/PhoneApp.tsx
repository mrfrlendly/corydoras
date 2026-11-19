import { useCallback, useEffect, useRef, useState } from 'react';
import { batchDrafts, blankForm, editForm, type WorkshopForm } from '../components/workshopForm';
import { errText } from './errText';
import PhoneReview, { type Card } from './PhoneReview';
import { connect, DISCONNECTED, keepsReadPending, newBatchId, PhoneError, phoneApi, type PhoneStatus } from './phoneApi';
import { preparePhoto } from './photo';
import { clearSession, loadSession, saveCards, saveFailure, saveSession } from './session';

type Screen = 'loading' | 'connect' | 'ready' | 'reading' | 'review' | 'saved';

const MODULE_OFF = 'Workshop inventory is off for this library. Open Settings to turn it on.';
const NO_PROVIDER = 'No analysis provider is set on the Mac. You can still add items by hand.';

export default function PhoneApp() {
  const [screen, setScreen] = useState<Screen>('loading');
  const [status, setStatus] = useState<PhoneStatus | null>(null);
  const [statusError, setStatusError] = useState('');
  const [disconnected, setDisconnected] = useState(false);
  const [photo, setPhoto] = useState<Blob | null>(null);
  const [batch, setBatch] = useState('');
  const [saveError, setSaveError] = useState('');
  const [preview, setPreview] = useState('');
  const [cards, setCards] = useState<Card[]>([]);
  const [error, setError] = useState('');
  const [readFailed, setReadFailed] = useState(false);
  const [saving, setSaving] = useState(false);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [saved, setSaved] = useState<{ count: number; library: string } | null>(null);
  const counter = useRef(0);
  const restored = useRef(false);

  const toCards = (forms: WorkshopForm[]) => forms.map(form => ({ id: counter.current++, form }));

  /** A 401 anywhere: the token is gone; the cards stay kept on the phone for after the new scan. */
  const fail = useCallback((e: unknown): string | null => {
    if (e instanceof PhoneError && e.status === 401) {
      setDisconnected(true);
      setScreen('connect');
      return null;
    }
    return errText(e);
  }, []);

  const loadStatus = useCallback(async () => {
    try {
      setStatus(await phoneApi.status());
      setStatusError('');
    } catch (e) {
      const text = fail(e);
      if (text) setStatusError(text);
    }
  }, [fail]);

  const read = useCallback(async (picture: Blob) => {
    setScreen('reading');
    setError('');
    setReadFailed(false);
    await saveCards(null);
    await saveFailure(null);
    try {
      const forms = (await phoneApi.read(picture)).map(editForm);
      await saveCards(forms);
      setCards(toCards(forms));
      if (forms.length === 0) setError('No items were found on this photo. Add them by hand, or take a closer photo.');
      setScreen('review');
    } catch (e) {
      const text = fail(e);
      if (!text) return;
      // A read that never reached the Mac stays pending (stored cards null): a reload asks again, and the Mac
      // answers from what it kept. A reload cancels the request, so this is the common case.
      // A read the Mac answered with an error may have been paid: keep the failure, so a reload does not read again.
      if (!keepsReadPending(e)) {
        await saveCards([]);
        await saveFailure(text);
      }
      setCards([]);
      setError(text);
      setReadFailed(true);
      setScreen('review');
    }
  }, [fail]);

  // Start: take the token, bring back a review in progress, then ask the Mac for its state.
  useEffect(() => {
    if (restored.current) return;
    restored.current = true;
    if (!connect()) {
      setScreen('connect');
      return;
    }
    void (async () => {
      const session = await loadSession();
      void loadStatus();
      if (!session) {
        setScreen('ready');
        return;
      }
      setPhoto(session.photo);
      setBatch(session.batch);
      if (session.failure) {
        setError(session.failure);
        setReadFailed(true);
      }
      if (session.cards === null) {
        void read(session.photo); // the Mac kept the result of a read that was running
      } else {
        setCards(toCards(session.cards));
        setScreen('review');
      }
    })();
  }, [loadStatus, read]);

  useEffect(() => {
    if (!photo) return setPreview('');
    const url = URL.createObjectURL(photo);
    setPreview(url);
    return () => URL.revokeObjectURL(url);
  }, [photo]);

  // Keep the cards on the phone while they are edited.
  // After a failed read, nothing is stored until a card exists, so the read stays pending.
  useEffect(() => {
    if (screen !== 'review' || (readFailed && cards.length === 0)) return;
    const timer = setTimeout(() => void saveCards(cards.map(card => card.form)), 300);
    return () => clearTimeout(timer);
  }, [cards, screen, readFailed]);

  async function pick(file: File | undefined) {
    if (!file) return;
    setError('');
    let picture: Blob;
    try {
      picture = await preparePhoto(file);
    } catch (e) {
      setError(errText(e));
      return;
    }
    const id = newBatchId();
    setPhoto(picture);
    setBatch(id);
    setSaved(null);
    setSaveError('');
    if (status && !status.provider_ready) {
      const forms = [blankForm()];
      await saveSession({ photo: picture, cards: forms, batch: id });
      setCards(toCards(forms));
      setError(NO_PROVIDER);
      setScreen('review');
      return;
    }
    await saveSession({ photo: picture, cards: null, batch: id });
    await read(picture);
  }

  async function discardAll() {
    await clearSession();
    setConfirmCancel(false);
    setCards([]);
    setPhoto(null);
    setError('');
    setSaveError('');
    setReadFailed(false);
    setScreen('ready');
  }

  async function save() {
    if (!photo || saving || cards.length === 0) return;
    setSaveError('');
    let drafts;
    try {
      drafts = batchDrafts(cards.map(card => card.form));
    } catch (e) {
      setSaveError(errText(e));
      return;
    }
    setSaving(true);
    try {
      // The same batch id on a retry: if the Mac saved before the answer was lost, it saves nothing new.
      const items = await phoneApi.save(drafts, photo, batch || newBatchId());
      await clearSession();
      setSaved({ count: items.length, library: status?.library || 'Corydoras' });
      setCards([]);
      setPhoto(null);
      setError('');
      setReadFailed(false);
      setScreen('saved');
      void loadStatus(); // new categories for the next photo
    } catch (e) {
      const text = fail(e);
      if (text) setSaveError(text);
    } finally {
      setSaving(false);
    }
  }

  const moduleOff = status !== null && !status.inventory_enabled;
  const photoButtons = <div className="actions">
    <label className={`btn primary ${moduleOff ? 'disabled' : ''}`}>
      <input className="sr-only" type="file" accept="image/*" capture="environment" disabled={moduleOff}
        onChange={e => { const file = e.currentTarget.files?.[0]; e.currentTarget.value = ''; void pick(file); }} />
      Take photo
    </label>
    <label className={`btn ${moduleOff ? 'disabled' : ''}`}>
      <input className="sr-only" type="file" accept="image/*" disabled={moduleOff}
        onChange={e => { const file = e.currentTarget.files?.[0]; e.currentTarget.value = ''; void pick(file); }} />
      Choose photo
    </label>
  </div>;

  return <main className={`phone ${screen === 'review' ? 'with-bar' : ''}`}>
    <header className="top">
      <h1>Workshop inventory</h1>
      {status && <p className="library">{status.library}</p>}
    </header>

    {screen === 'loading' && <p className="muted" role="status">Connecting…</p>}

    {screen === 'connect' && <section className="panel">
      <h2>Connect this phone</h2>
      <p>{disconnected ? DISCONNECTED : 'Open Corydoras on your Mac, go to Settings › Phone, and scan the code.'}</p>
    </section>}

    {screen === 'ready' && <section className="panel">
      {statusError && <><p className="error" role="alert">{statusError}</p>
        <button type="button" className="btn" onClick={() => void loadStatus()}>Try again</button></>}
      {moduleOff && <p className="error" role="alert">{MODULE_OFF}</p>}
      {status && !status.provider_ready && !moduleOff && <p className="muted">{NO_PROVIDER}</p>}
      <p>Photograph a label, a size chart or a group of products. You review every item before anything is saved.</p>
      {error && <p className="error" role="alert">{error}</p>}
      {photoButtons}
    </section>}

    {screen === 'reading' && <section className="panel">
      {preview && <img className="photo" alt="The label photo" src={preview} />}
      <p role="status">Reading the label… this can take up to a minute.</p>
    </section>}

    {screen === 'review' && <>
      {preview && <img className="photo" alt="The label photo" src={preview} />}
      {error && <p className="error" role="alert">{error}</p>}
      {readFailed && photo && <button type="button" className="btn" onClick={() => void read(photo)}>Read the label again</button>}
      {cards.length > 0 && <p className="muted">Quantity on hand is blank: enter it, or press “I bought one pack”.</p>}
      <PhoneReview cards={cards} categories={status?.categories ?? []} disabled={saving}
        onChange={(id, form) => setCards(previous => previous.map(card => card.id === id ? { ...card, form } : card))}
        onDiscard={id => setCards(previous => previous.filter(card => card.id !== id))} />
      <button type="button" className="btn" disabled={saving}
        onClick={() => setCards(previous => [...previous, ...toCards([blankForm()])])}>Add an item by hand</button>
      <div className="bar">
        {saveError && !confirmCancel && <p className="error bar-error" role="alert">{saveError}</p>}
        {confirmCancel ? <div className="confirm" role="alertdialog" aria-label="Discard the unsaved items?">
          <p>Discard {cards.length === 1 ? 'the 1 unsaved item' : `the ${cards.length} unsaved items`} from this photo?</p>
          <div className="actions">
            <button type="button" className="btn" autoFocus onClick={() => setConfirmCancel(false)}>Keep reviewing</button>
            <button type="button" className="btn danger" onClick={() => void discardAll()}>Discard items</button>
          </div>
        </div> : <div className="actions">
          <button type="button" className="btn" disabled={saving}
            onClick={() => cards.length ? setConfirmCancel(true) : void discardAll()}>Cancel</button>
          <button type="button" className="btn primary grow" disabled={saving || cards.length === 0} onClick={() => void save()}>
            {saving ? 'Saving…' : cards.length === 0 ? 'Save' : cards.length === 1 ? 'Save 1 item' : `Save ${cards.length} items`}
          </button>
        </div>}
      </div>
    </>}

    {screen === 'saved' && saved && <section className="panel">
      <p className="done" role="status">{saved.count === 1 ? `1 item saved to ${saved.library}.` : `${saved.count} items saved to ${saved.library}.`}</p>
      {error && <p className="error" role="alert">{error}</p>}
      <div className="actions">
        <label className="btn primary">
          <input className="sr-only" type="file" accept="image/*" capture="environment"
            onChange={e => { const file = e.currentTarget.files?.[0]; e.currentTarget.value = ''; void pick(file); }} />
          Take another photo
        </label>
      </div>
    </section>}
  </main>;
}
