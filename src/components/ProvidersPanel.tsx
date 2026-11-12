import { useCallback, useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
  Check,
  Cpu,
  ExternalLink,
  Eye,
  EyeOff,
  KeyRound,
  ListRestart,
  Loader,
  LogIn,
  Pencil,
  PlugZap,
  Plus,
  Server,
  Trash2,
  X,
} from 'lucide-react';
import { AccountView, AiConfig, AiView, api, CHATGPT_USAGE_URL, errText, ModelInfo, Preset, ProviderProfile } from '../api';
import { useToast } from './Toast';
import { useConfirmRow } from './useConfirmRow';

function VisionChip({ p }: { p: ProviderProfile }) {
  if (p.vision === true)
    return (
      <span className="chip chip-ok">
        <Eye size={11} /> Reads images
      </span>
    );
  if (p.vision === false)
    return (
      <span className="chip chip-warn">
        <EyeOff size={11} /> No images
      </span>
    );
  return <span className="chip">Not tested</span>;
}

/** "Using ChatGPT plan" with the link to the usage settings of ChatGPT. */
function PlanInUse() {
  return (
    <>
      <span className="chip">Using ChatGPT plan</span>
      <button className="link-btn small" onClick={() => openUrl(CHATGPT_USAGE_URL).catch(() => {})}>
        <ExternalLink size={11} /> Manage usage
      </button>
    </>
  );
}

/** The row that shows while the app waits for the browser. */
function WaitingForBrowser({ onCancel }: { onCancel: () => void }) {
  return (
    <>
      <Loader size={13} className="spin" />
      <span className="small" role="status">
        Finish the sign-in in your browser. Corydoras waits for it.
      </span>
      <button className="btn small" onClick={onCancel}>
        Cancel
      </button>
    </>
  );
}

function Editor({
  draft: initial,
  preset,
  hasKey,
  ready,
  account,
  inUse,
  canRemove,
  onSave,
  onCancel,
  onRemove,
  onKeyChanged,
  onTested,
  onSignedIn,
  onView,
}: {
  draft: ProviderProfile;
  preset: Preset | undefined;
  hasKey: boolean;
  /** The provider can pay for a call: it has its key, needs none, or the ChatGPT plan is signed in. */
  ready: boolean;
  /** The ChatGPT account of the app. */
  account: AccountView;
  /** The provider runs the analysis or writes the language document. */
  inUse: boolean;
  canRemove: boolean;
  /** Saves the profile and gives it back as saved (the app trims names and addresses). */
  onSave: (p: ProviderProfile, quiet?: boolean) => Promise<ProviderProfile | undefined>;
  onCancel: () => void;
  onRemove: () => void;
  onKeyChanged: () => void;
  onTested: () => void;
  /** A sign-in finished. `welcome`: show the notice about plan usage. */
  onSignedIn: (view: AiView, welcome: boolean) => void;
  onView: (view: AiView) => void;
}) {
  const toast = useToast();
  const [d, setD] = useState<ProviderProfile>(initial);
  const [key, setKey] = useState('');
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [confirmKey, setConfirmKey] = useState(false);
  const [confirmOut, setConfirmOut] = useState(false);
  const [signing, setSigning] = useState(false);
  const signInCancelled = useRef(false);
  const removeTrigger = useConfirmRow(confirmRemove, () => setConfirmRemove(false));
  const keyTrigger = useConfirmRow(confirmKey, () => setConfirmKey(false));
  const outTrigger = useConfirmRow(confirmOut, () => setConfirmOut(false));
  const changed = key.trim() !== '' || JSON.stringify(d) !== JSON.stringify(initial);
  const set = (patch: Partial<ProviderProfile>) => setD((x) => ({ ...x, ...patch }));
  const listId = `models-${d.id}`;
  /** A ChatGPT account pays, not a key: the address, the output limit and the prices do not apply. */
  const plan = preset?.sign_in === 'chatgpt';

  const saveKey = async () => {
    try {
      const saved = await onSave(d); // The profile must exist before its key.
      if (saved) setD(saved);
      await api.setProviderKey(d.id, key);
      setKey('');
      toast.info(`Key for ${d.name} saved.`);
      onKeyChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const removeKey = async () => {
    try {
      await api.clearProviderKey(d.id);
      setConfirmKey(false);
      toast.info(`Key for ${d.name} removed.`);
      onKeyChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  /** Asks the provider for its models. `quiet`: no message about the count. */
  const loadModels = async (profile: ProviderProfile = d, quiet = false): Promise<ModelInfo[]> => {
    setLoadingModels(true);
    try {
      const list = await api.listModels(profile);
      setModels(list);
      if (!quiet) toast.info(list.length ? `${list.length} models. Type in the Model field to filter them.` : 'The provider listed no models.');
      return list;
    } catch (e) {
      toast.error(errText(e));
      return [];
    } finally {
      setLoadingModels(false);
    }
  };

  const signIn = async (fresh: boolean) => {
    setSigning(true);
    signInCancelled.current = false;
    try {
      const saved = (await onSave(d, true)) ?? d; // The profile must exist before its sign-in.
      setD(saved);
      const r = await api.providerSignIn(saved.id, fresh);
      onSignedIn(r.view, r.welcome);
      if (plan) {
        toast.info(`Signed in to ChatGPT${r.view.account.email ? ` as ${r.view.account.email}` : ''}.`);
        if (r.view.account.plan) {
          const list = await loadModels(saved, true);
          // A provider without a model starts with the first model that ChatGPT lists, so it can work at once.
          if (!saved.model.trim() && list.length > 0) {
            await onSave({ ...saved, model: list[0].id, vision: null }, true);
            toast.info(`${list[0].name || list[0].id} is the model now: the first one of your plan. You can choose another.`);
          }
        }
      } else {
        toast.info(`Signed in. The key of your account is saved for ${saved.name}.`);
      }
    } catch (e) {
      if (!signInCancelled.current) toast.error(errText(e));
    } finally {
      setSigning(false);
    }
  };

  const cancelSignIn = () => {
    signInCancelled.current = true;
    api.cancelSignIn().catch(() => {});
  };

  const signOut = async () => {
    try {
      const r = await api.providerSignOut();
      setConfirmOut(false);
      onView(r.view);
      toast.info(
        r.confirmed
          ? 'Signed out of ChatGPT.'
          : 'Signed out on this Mac. ChatGPT did not confirm it: you can disconnect Corydoras in your ChatGPT settings.',
      );
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const test = async () => {
    setTesting(true);
    setResult(null);
    try {
      const r = await api.checkProvider(d);
      set({ vision: r.vision });
      setResult({ ok: true, text: r.message });
      onTested();
    } catch (e) {
      setResult({ ok: false, text: errText(e) });
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="prov-editor">
      <div className="grid2">
        <label className="field">
          <span>Name</span>
          <input value={d.name} onChange={(e) => set({ name: e.target.value })} />
        </label>
        {!plan && (
          <label className="field">
            <span>Format</span>
            <select className="select" value={d.api} onChange={(e) => set({ api: e.target.value as ProviderProfile['api'] })} disabled={d.kind !== 'custom'}>
              <option value="openai">OpenAI-compatible (/chat/completions)</option>
              <option value="anthropic">Anthropic Messages (/messages)</option>
            </select>
          </label>
        )}
        {!plan && (
          <label className="field span2">
            <span>Address (base URL)</span>
            <input className="mono" value={d.base_url} onChange={(e) => set({ base_url: e.target.value })} placeholder="https://…/v1" spellCheck={false} autoCorrect="off" autoCapitalize="off" />
          </label>
        )}
        {plan && (
          <label className="field">
            <span>Thinking (language document only)</span>
            <select className="select" value={d.thinking} onChange={(e) => set({ thinking: e.target.value as ProviderProfile['thinking'] })}>
              <option value="off">Off — the model's own setting</option>
              <option value="low">Low — less usage of your plan</option>
              <option value="high">High — most careful, most usage</option>
            </select>
          </label>
        )}
        <label className="field span2">
          <span>Model</span>
          <div className="row nowrap">
            <input
              className="grow-input mono"
              list={listId}
              value={d.model}
              onChange={(e) => set({ model: e.target.value, vision: e.target.value === initial.model ? initial.vision : null })}
              placeholder={plan ? 'Sign in, then load the models of your plan' : 'Type a model ID, or load the list'}
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
            />
            <button className="btn small" onClick={() => loadModels()} disabled={loadingModels || (plan && !account.plan)} title="Ask the provider for its models">
              {loadingModels ? <Loader size={13} className="spin" /> : <ListRestart size={13} />} Load models
            </button>
          </div>
          <datalist id={listId}>
            {models.map((m) => (
              <option key={m.id} value={m.id} label={m.name || undefined} />
            ))}
          </datalist>
        </label>
        {!plan && (
          <>
            <label className="field">
              <span>Thinking (language document only)</span>
              <select className="select" value={d.thinking} onChange={(e) => set({ thinking: e.target.value as ProviderProfile['thinking'] })}>
                <option value="off">Off — fastest, lowest cost</option>
                <option value="low">Low — recommended for reasoning models</option>
                <option value="high">High — most careful, most tokens</option>
              </select>
            </label>
            <label className="field">
              <span>Output limit (thousand tokens)</span>
              <input
                type="number"
                min={1}
                max={200}
                value={Math.round(d.max_output / 1000)}
                onChange={(e) => set({ max_output: Math.max(1, Number(e.target.value) || 1) * 1000 })}
              />
            </label>
            <label className="field">
              <span>Price of input (US dollars for a million tokens, optional)</span>
              <input type="number" min={0} step={0.01} value={d.price_in || ''} placeholder="Not set" onChange={(e) => set({ price_in: Math.max(0, Number(e.target.value) || 0) })} />
            </label>
            <label className="field">
              <span>Price of output (US dollars for a million tokens, optional)</span>
              <input type="number" min={0} step={0.01} value={d.price_out || ''} placeholder="Not set" onChange={(e) => set({ price_out: Math.max(0, Number(e.target.value) || 0) })} />
            </label>
          </>
        )}
      </div>
      {!plan && <p className="muted small">With prices, the estimate before each pass shows money next to the tokens. Take them from the price list of the provider.</p>}
      {preset?.note && <p className="muted small">{preset.note}</p>}

      {plan && (
        <div className="field plan-box">
          <span>
            ChatGPT account{' '}
            {account.plan ? (
              <span className="chip chip-ok">Signed in</span>
            ) : account.signed_in ? (
              <span className="chip chip-warn">Plan not allowed</span>
            ) : (
              <span className="chip chip-warn">Not signed in</span>
            )}
          </span>
          {account.plan ? (
            <div className="row">
              <span className="small">{account.email || 'Your ChatGPT account'}</span>
              <PlanInUse />
              <div className="spacer" />
              {!confirmOut && (
                <button ref={outTrigger} className="btn small ghost-danger" onClick={() => setConfirmOut(true)}>
                  Sign out…
                </button>
              )}
            </div>
          ) : (
            <>
              <p className="small plan-offer">
                <b>Use your ChatGPT plan.</b> Complete eligible AI requests in this app with usage included in your ChatGPT plan or credits balance.
              </p>
              {account.signed_in && (
                <p className="small error-text">
                  This sign-in does not allow the use of your plan. Continue again and allow it in ChatGPT, or use a provider with an API key.
                </p>
              )}
              <div className="row">
                {signing ? (
                  <WaitingForBrowser onCancel={cancelSignIn} />
                ) : (
                  <>
                    <button className="btn chatgpt-btn" onClick={() => signIn(false)}>
                      Continue with ChatGPT
                    </button>
                    {account.known && (
                      <button className="link-btn small" onClick={() => signIn(true)} title="Register Corydoras for another ChatGPT account">
                        Use another account
                      </button>
                    )}
                    {account.signed_in && !confirmOut && (
                      <button ref={outTrigger} className="btn small ghost-danger" onClick={() => setConfirmOut(true)}>
                        Sign out…
                      </button>
                    )}
                  </>
                )}
              </div>
              {account.known && account.email && !signing && <span className="muted small">Continues as {account.email}.</span>}
            </>
          )}
          {confirmOut && (
            <div className="row confirm-row">
              <button className="btn small danger" onClick={signOut}>
                Sign out of ChatGPT
              </button>
              <button className="btn small" autoFocus onClick={() => setConfirmOut(false)}>
                Stay signed in
              </button>
              <span className="muted small">
                {inUse ? 'The queue stops until you sign in again.' : 'Every ChatGPT plan provider stops until you sign in again.'}
              </span>
            </div>
          )}
          <p className="muted small">
            One ChatGPT account serves every ChatGPT plan provider in Corydoras. The sign-in happens in your browser; Corydoras gets no access to your ChatGPT
            conversations. Plan usage is separate from an OpenAI API key, and Corydoras itself charges nothing.
          </p>
        </div>
      )}

      {!plan && d.needs_key && (
        <div className="field">
          <span>
            API key {hasKey ? <span className="chip chip-ok">Saved</span> : <span className="chip chip-warn">No key</span>}
          </span>
          {preset?.sign_in === 'openrouter' && (
            <div className="row signin-row">
              {signing ? (
                <WaitingForBrowser onCancel={cancelSignIn} />
              ) : (
                <>
                  <button className="btn small" onClick={() => signIn(false)}>
                    <LogIn size={13} /> Sign in with OpenRouter
                  </button>
                  <span className="muted small">
                    {hasKey
                      ? 'Makes a new key in your OpenRouter account and puts it in place of the saved one.'
                      : 'Opens your browser. OpenRouter makes a key for Corydoras in your account; you paste nothing.'}
                  </span>
                </>
              )}
            </div>
          )}
          <div className="row nowrap">
            <input
              className="grow-input"
              type="password"
              placeholder={hasKey ? 'Paste a new key to replace it' : `${preset?.sign_in === 'openrouter' ? 'Or paste' : 'Paste'} your ${preset?.name ?? ''} API key`}
              value={key}
              onChange={(e) => setKey(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && key.trim() && saveKey()}
            />
            <button className="btn small primary" disabled={!key.trim()} onClick={saveKey}>
              <KeyRound size={13} /> Save key
            </button>
            {hasKey && !confirmKey && (
              <button ref={keyTrigger} className="btn small ghost-danger" onClick={() => setConfirmKey(true)}>
                Remove key…
              </button>
            )}
          </div>
          {hasKey && confirmKey && (
            <div className="row confirm-row">
              <button className="btn small danger" onClick={removeKey}>
                Remove key
              </button>
              <button className="btn small" autoFocus onClick={() => setConfirmKey(false)}>
                Keep
              </button>
              <span className="muted small">
                {inUse ? 'The queue stops until you paste a key again.' : 'This provider stops working until you paste a key again.'}
              </span>
            </div>
          )}
          {preset?.key_url && (
            <button className="link-btn small" onClick={() => openUrl(preset.key_url).catch(() => {})}>
              <ExternalLink size={11} /> {preset.sign_in === 'openrouter' ? 'See or delete your keys' : 'Make a key'} at {new URL(preset.key_url).host}
            </button>
          )}
        </div>
      )}
      {!plan && !d.needs_key && (
        <label className="check small">
          <input type="checkbox" checked={d.needs_key} onChange={(e) => set({ needs_key: e.target.checked })} /> This server needs an API key
        </label>
      )}

      <div className="row">
        <button className="btn small" onClick={test} disabled={testing || (plan ? !ready : d.needs_key && !hasKey) || !d.model.trim()}>
          {testing ? <Loader size={13} className="spin" /> : <PlugZap size={13} />} {testing ? 'Testing…' : 'Test'}
        </button>
        {result && <span className={`small ${result.ok ? '' : 'error-text'}`}>{result.text}</span>}
      </div>

      <div className="row prov-actions">
        <button
          className="btn small primary"
          onClick={() =>
            onSave(d)
              .then((saved) => saved && setD(saved))
              .catch((e) => toast.error(errText(e)))
          }
        >
          <Check size={13} /> Save provider
        </button>
        <button className="btn small" onClick={onCancel}>
          {changed ? 'Cancel' : 'Close'}
        </button>
        <div className="spacer" />
        {canRemove &&
          (confirmRemove ? (
            <>
              <button className="btn small danger" onClick={onRemove}>
                <Trash2 size={13} /> {plan ? 'Remove provider' : 'Remove provider and key'}
              </button>
              <button className="btn small" autoFocus onClick={() => setConfirmRemove(false)}>
                Keep
              </button>
            </>
          ) : (
            <button ref={removeTrigger} className="btn small ghost-danger" onClick={() => setConfirmRemove(true)}>
              <Trash2 size={13} /> Remove…
            </button>
          ))}
      </div>
    </div>
  );
}

/** The notice of the first sign-in with a ChatGPT plan. It shows one time. */
function PlanWelcome({ onClose }: { onClose: () => void }) {
  return (
    <div className="modal-scrim">
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="plan-welcome-title"
        aria-describedby="plan-welcome-text"
        onKeyDown={(e) => {
          if (e.key === 'Escape') onClose();
          // One control: the focus stays on it.
          if (e.key === 'Tab') e.preventDefault();
        }}
      >
        <h3 id="plan-welcome-title">You’re using your ChatGPT plan</h3>
        <p id="plan-welcome-text">Eligible usage in this app uses your ChatGPT plan. Manage usage in your ChatGPT settings.</p>
        <div className="row modal-actions">
          <button className="btn primary" autoFocus onClick={onClose}>
            Got it
          </button>
        </div>
      </div>
    </div>
  );
}

export default function ProvidersPanel({ onChanged }: { onChanged: () => void }) {
  const toast = useToast();
  const [view, setView] = useState<AiView | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [addKind, setAddKind] = useState('');
  const [welcome, setWelcome] = useState(false);
  // The newest view. A sign-in waits for the browser for minutes, and what it saves afterwards
  // must build on the providers as they are then, not as they were when it started.
  const newest = useRef<AiView | null>(null);
  const show = useCallback((v: AiView) => {
    newest.current = v;
    setView(v);
  }, []);

  const load = useCallback(() => {
    api
      .getAi()
      .then(show)
      .catch((e) => toast.error(errText(e)));
  }, [toast, show]);
  useEffect(load, [load]);

  const save = async (config: AiConfig, quiet = false) => {
    const v = await api.saveAi(config);
    show(v);
    if (!quiet) toast.info('Providers saved.');
    onChanged();
    return v;
  };

  if (!view) return null;
  const { config, presets, keys, account } = view;
  const presetOf = (kind: string) => presets.find((p) => p.kind === kind);
  const analysis = config.profiles.find((p) => p.id === config.analysis_profile);
  const synthesis = config.profiles.find((p) => p.id === config.synthesis_profile);
  const onPlan = (p: ProviderProfile | undefined) => !!p && presetOf(p.kind)?.sign_in === 'chatgpt';
  const planInUse = account.plan && (onPlan(analysis) || onPlan(synthesis));

  const add = async () => {
    if (!addKind) return;
    try {
      const p = await api.newProfile(addKind);
      await save({ ...config, profiles: [...config.profiles, p] }, true);
      setEditing(p.id);
      setAddKind('');
    } catch (e) {
      toast.error(errText(e));
    }
  };

  return (
    <section className="panel" id="settings-providers" tabIndex={-1}>
      <h3>
        <Cpu size={15} /> Model providers
      </h3>
      <p className="muted small">
        Use any provider with your own key, sign in to OpenRouter or to your ChatGPT plan, or use a model on this Mac. Keys and sign-ins are files that only your
        user account can read. Choose one provider for the image and source analysis and one for the language document; they can be the same.
      </p>

      <div className="grid2 prov-roles">
        <label className="field">
          <span>Images and sources (needs a model that reads images)</span>
          <select className="select" value={config.analysis_profile} onChange={(e) => save({ ...config, analysis_profile: e.target.value })}>
            {config.profiles.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name} · {p.model || 'no model'}
                {p.vision === false ? ' (no images)' : ''}
              </option>
            ))}
          </select>
          {analysis?.vision === false && <span className="small error-text">This model does not read images. Choose another for the analysis.</span>}
        </label>
        <label className="field">
          <span>Language document</span>
          <select className="select" value={config.synthesis_profile} onChange={(e) => save({ ...config, synthesis_profile: e.target.value })}>
            {config.profiles.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name} · {p.model || 'no model'} · thinking {p.thinking}
              </option>
            ))}
          </select>
        </label>
      </div>
      {planInUse && (
        <div className="row plan-line">
          <PlanInUse />
        </div>
      )}

      <div className="prov-list">
        {config.profiles.map((p) => {
          const preset = presetOf(p.kind);
          const open = editing === p.id;
          const plan = preset?.sign_in === 'chatgpt';
          const roles = [config.analysis_profile === p.id && 'Analysis', config.synthesis_profile === p.id && 'Language document'].filter(Boolean);
          return (
            <div key={p.id} className={`prov-card ${open ? 'open' : ''}`}>
              <div className="prov-head" onClick={() => setEditing(open ? null : p.id)}>
                <span className="prov-icon">{preset?.local ? <Server size={16} /> : <Cpu size={16} />}</span>
                <div className="prov-title">
                  <div className="prov-name">{p.name}</div>
                  <div className="mono small muted">{p.model || 'Choose a model'}</div>
                </div>
                <div className="prov-chips">
                  {roles.map((r) => (
                    <span key={r as string} className="chip">
                      {r}
                    </span>
                  ))}
                  <VisionChip p={p} />
                  {p.needs_key && !keys[p.id] && <span className="chip chip-warn">No key</span>}
                  {plan && (keys[p.id] ? <span className="chip">Using ChatGPT plan</span> : <span className="chip chip-warn">Not signed in</span>)}
                </div>
                <button className="icon-btn" title={open ? 'Close' : 'Edit'} aria-expanded={open} aria-label={`${open ? 'Close' : 'Edit'} ${p.name}`}>
                  {open ? <X size={14} /> : <Pencil size={14} />}
                </button>
              </div>
              {open && (
                <Editor
                  key={p.id + p.model}
                  draft={p}
                  preset={preset}
                  hasKey={!!keys[p.id] && p.needs_key}
                  ready={!!keys[p.id]}
                  account={account}
                  inUse={roles.length > 0}
                  canRemove={config.profiles.length > 1}
                  onCancel={() => setEditing(null)}
                  onSave={async (np, quiet) => {
                    const now = newest.current?.config ?? config;
                    const v = await save({ ...now, profiles: now.profiles.map((x) => (x.id === np.id ? np : x)) }, quiet);
                    return v.config.profiles.find((x) => x.id === np.id);
                  }}
                  onRemove={async () => {
                    try {
                      await save({ ...config, profiles: config.profiles.filter((x) => x.id !== p.id) });
                      setEditing(null);
                    } catch (e) {
                      toast.error(errText(e));
                    }
                  }}
                  onTested={load}
                  onKeyChanged={() => {
                    load();
                    onChanged();
                  }}
                  onSignedIn={(v, first) => {
                    show(v);
                    if (first) setWelcome(true);
                    onChanged();
                  }}
                  onView={(v) => {
                    show(v);
                    onChanged();
                  }}
                />
              )}
            </div>
          );
        })}
      </div>

      <div className="row">
        <select className="select" value={addKind} onChange={(e) => setAddKind(e.target.value)} aria-label="Provider to add">
          <option value="">Add a provider…</option>
          {presets.map((p) => (
            <option key={p.kind} value={p.kind}>
              {p.name}
            </option>
          ))}
        </select>
        <button className="btn small" disabled={!addKind} onClick={add}>
          <Plus size={13} /> Add
        </button>
      </div>

      {welcome && <PlanWelcome onClose={() => setWelcome(false)} />}
    </section>
  );
}
