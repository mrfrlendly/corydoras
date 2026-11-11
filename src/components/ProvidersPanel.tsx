import { useCallback, useEffect, useState } from 'react';
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
  Pencil,
  PlugZap,
  Plus,
  Server,
  Trash2,
  X,
} from 'lucide-react';
import { AiConfig, AiView, api, errText, Preset, ProviderProfile } from '../api';
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

function Editor({
  draft: initial,
  preset,
  hasKey,
  inUse,
  canRemove,
  onSave,
  onCancel,
  onRemove,
  onKeyChanged,
  onTested,
}: {
  draft: ProviderProfile;
  preset: Preset | undefined;
  hasKey: boolean;
  /** The provider runs the analysis or writes the language document. */
  inUse: boolean;
  canRemove: boolean;
  /** Saves the profile and gives it back as saved (the app trims names and addresses). */
  onSave: (p: ProviderProfile) => Promise<ProviderProfile | undefined>;
  onCancel: () => void;
  onRemove: () => void;
  onKeyChanged: () => void;
  onTested: () => void;
}) {
  const toast = useToast();
  const [d, setD] = useState<ProviderProfile>(initial);
  const [key, setKey] = useState('');
  const [models, setModels] = useState<string[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [confirmKey, setConfirmKey] = useState(false);
  const removeTrigger = useConfirmRow(confirmRemove, () => setConfirmRemove(false));
  const keyTrigger = useConfirmRow(confirmKey, () => setConfirmKey(false));
  const changed = key.trim() !== '' || JSON.stringify(d) !== JSON.stringify(initial);
  const set = (patch: Partial<ProviderProfile>) => setD((x) => ({ ...x, ...patch }));
  const listId = `models-${d.id}`;

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

  const loadModels = async () => {
    setLoadingModels(true);
    try {
      const list = await api.listModels(d);
      setModels(list);
      toast.info(list.length ? `${list.length} models. Type in the Model field to filter them.` : 'The provider listed no models.');
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setLoadingModels(false);
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
        <label className="field">
          <span>Format</span>
          <select className="select" value={d.api} onChange={(e) => set({ api: e.target.value as ProviderProfile['api'] })} disabled={d.kind !== 'custom'}>
            <option value="openai">OpenAI-compatible (/chat/completions)</option>
            <option value="anthropic">Anthropic Messages (/messages)</option>
          </select>
        </label>
        <label className="field span2">
          <span>Address (base URL)</span>
          <input className="mono" value={d.base_url} onChange={(e) => set({ base_url: e.target.value })} placeholder="https://…/v1" />
        </label>
        <label className="field span2">
          <span>Model</span>
          <div className="row nowrap">
            <input
              className="grow-input mono"
              list={listId}
              value={d.model}
              onChange={(e) => set({ model: e.target.value, vision: e.target.value === initial.model ? initial.vision : null })}
              placeholder="Type a model ID, or load the list"
            />
            <button className="btn small" onClick={loadModels} disabled={loadingModels} title="Ask the provider for its models">
              {loadingModels ? <Loader size={13} className="spin" /> : <ListRestart size={13} />} Load models
            </button>
          </div>
          <datalist id={listId}>
            {models.map((m) => (
              <option key={m} value={m} />
            ))}
          </datalist>
        </label>
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
      </div>
      <p className="muted small">With prices, the estimate before each pass shows money next to the tokens. Take them from the price list of the provider.</p>
      {preset?.note && <p className="muted small">{preset.note}</p>}

      {d.needs_key && (
        <div className="field">
          <span>
            API key {hasKey ? <span className="chip chip-ok">Saved</span> : <span className="chip chip-warn">No key</span>}
          </span>
          <div className="row nowrap">
            <input
              className="grow-input"
              type="password"
              placeholder={hasKey ? 'Paste a new key to replace it' : `Paste your ${preset?.name ?? ''} API key`}
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
              <ExternalLink size={11} /> Make a key at {new URL(preset.key_url).host}
            </button>
          )}
        </div>
      )}
      {!d.needs_key && (
        <label className="check small">
          <input type="checkbox" checked={d.needs_key} onChange={(e) => set({ needs_key: e.target.checked })} /> This server needs an API key
        </label>
      )}

      <div className="row">
        <button className="btn small" onClick={test} disabled={testing || (d.needs_key && !hasKey) || !d.model.trim()}>
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
                <Trash2 size={13} /> Remove provider and key
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

export default function ProvidersPanel({ onChanged }: { onChanged: () => void }) {
  const toast = useToast();
  const [view, setView] = useState<AiView | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [addKind, setAddKind] = useState('');

  const load = useCallback(() => {
    api
      .getAi()
      .then(setView)
      .catch((e) => toast.error(errText(e)));
  }, [toast]);
  useEffect(load, [load]);

  const save = async (config: AiConfig, quiet = false) => {
    const v = await api.saveAi(config);
    setView(v);
    if (!quiet) toast.info('Providers saved.');
    onChanged();
    return v;
  };

  if (!view) return null;
  const { config, presets, keys } = view;
  const presetOf = (kind: string) => presets.find((p) => p.kind === kind);
  const analysis = config.profiles.find((p) => p.id === config.analysis_profile);

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
        Use any provider with your own key, or a model on this Mac. Each provider keeps its key in a file that only your user account can read. Choose one provider for
        the image and source analysis and one for the language document; they can be the same.
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

      <div className="prov-list">
        {config.profiles.map((p) => {
          const preset = presetOf(p.kind);
          const open = editing === p.id;
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
                  inUse={roles.length > 0}
                  canRemove={config.profiles.length > 1}
                  onCancel={() => setEditing(null)}
                  onSave={async (np) => {
                    const v = await save({ ...config, profiles: config.profiles.map((x) => (x.id === np.id ? np : x)) });
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
    </section>
  );
}
