import { useEffect, useState } from 'react';
import { api, AppInfo, errText, fmtTokens, Settings, StatusReport } from '../api';
import { inventoryApi } from '../inventoryApi';
import { Boxes, Bot, SlidersHorizontal } from 'lucide-react';
import ProvidersPanel from './ProvidersPanel';
import StoragePanel from './StoragePanel';
import CapturePanel from './CapturePanel';
import BridgePanel from './BridgePanel';
import { useToast } from './Toast';

interface Props {
  info: AppInfo;
  onInfo: (i: AppInfo) => void;
  status: StatusReport | null;
  onChanged: () => void;
}

/** The sections of the page, in order, for the links under the title. */
const SECTIONS: [id: string, label: string][] = [
  ['settings-providers', 'Providers'],
  ['settings-modules', 'Modules'],
  ['settings-agents', 'Agents'],
  ['settings-pipeline', 'Pipeline'],
  ['settings-capture', 'Screen capture'],
  ['settings-extension', 'Browser extension'],
  ['settings-library', 'Library'],
];

function jump(id: string) {
  const el = document.getElementById(id);
  if (!el) return;
  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  el.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', block: 'start' });
  el.focus({ preventScroll: true });
}

async function copy(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export default function SettingsView({ info, onInfo, status, onChanged }: Props) {
  const toast = useToast();
  const [moduleBusy, setModuleBusy] = useState(false);
  const setInventory = async (value: boolean) => {
    setModuleBusy(true);
    try {
      onInfo(await inventoryApi.setEnabled(value));
    } catch (error) {
      toast.error(errText(error));
    } finally {
      setModuleBusy(false);
    }
  };
  const [settings, setSettings] = useState<Settings | null>(null);
  // The pipeline as saved; the section waits for its Save button.
  const [saved, setSaved] = useState<Settings | null>(null);

  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        setSettings(s);
        setSaved(s);
      })
      .catch((e) => toast.error(errText(e)));
  }, [toast]);
  const pipelineDirty = settings !== null && JSON.stringify(settings) !== JSON.stringify(saved);
  const saveSettings = async () => {
    if (!settings) return;
    try {
      const s = await api.saveSettings(settings);
      setSettings(s);
      setSaved(s);
      toast.info('Pipeline saved.');
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const mcpConfig = JSON.stringify(
    {
      mcpServers: {
        corydoras: {
          command: info.mcp_command ?? '~/.local/bin/corydoras-mcp',
          args: [],
        },
      },
    },
    null,
    2,
  );
  const claudeCodeCmd = `claude mcp add corydoras -- ${info.mcp_command ?? '~/.local/bin/corydoras-mcp'}`;

  return (
    <div className="settings">
      <header className="lang-head" data-tauri-drag-region>
        <div className="lang-title">
          <div className="eyebrow">Corydoras {info.version}</div>
          <h1>Settings</h1>
          <p className="lang-desc muted">Most choices apply at once. A section with a Save button waits for it.</p>
          <nav className="page-jump" aria-label="Settings sections">
            {SECTIONS.map(([id, label]) => (
              <button key={id} className="link-btn" onClick={() => jump(id)}>
                {label}
              </button>
            ))}
          </nav>
        </div>
      </header>

      <div className="settings-body">
        <ProvidersPanel onChanged={onChanged} />

        <section className="panel" id="settings-modules" tabIndex={-1}>
          <h3><Boxes size={15} /> Modules</h3>
          <label className="check">
            <input type="checkbox" checked={info.inventory_enabled} disabled={moduleBusy}
              aria-describedby="workshop-help" onChange={(e) => setInventory(e.target.checked)} />
            Workshop inventory
          </label>
          <p id="workshop-help" className="muted small">
            Show a separate inventory tab for this library. Turn it off to hide the tab and keep your items.
          </p>
        </section>

        <section className="panel" id="settings-agents" tabIndex={-1}>
          <h3>
            <Bot size={15} /> Agents (MCP server)
          </h3>
          {info.mcp_command ? (
            <p className="small">
              Server binary: <span className="mono">{info.mcp_command}</span>
            </p>
          ) : (
            <p className="small error-text">The MCP server binary is not installed. Run scripts/setup-mac.sh again.</p>
          )}
          <p className="muted small">
            For Claude Desktop, add this to <span className="mono">~/Library/Application Support/Claude/claude_desktop_config.json</span>, then
            restart Claude. The server reads the library that the app uses, also after a move:
          </p>
          <pre className="json">{mcpConfig}</pre>
          <div className="row">
            <button className="btn small" onClick={async () => toast.info((await copy(mcpConfig)) ? 'Copied.' : 'Copy failed.')}>
              Copy JSON
            </button>
          </div>
          <p className="muted small">For Claude Code, run:</p>
          <pre className="json">{claudeCodeCmd}</pre>
          <div className="row">
            <button className="btn small" onClick={async () => toast.info((await copy(claudeCodeCmd)) ? 'Copied.' : 'Copy failed.')}>
              Copy command
            </button>
          </div>
        </section>

        {settings && (
          <section className="panel" id="settings-pipeline" tabIndex={-1}>
            <h3>
              <SlidersHorizontal size={15} /> Pipeline
            </h3>
            <div className="grid2">
              <label className="field">
                <span>Wait after the last change before synthesis (seconds)</span>
                <input
                  type="number"
                  min={5}
                  max={3600}
                  value={settings.debounce_secs}
                  onChange={(e) => setSettings({ ...settings, debounce_secs: Number(e.target.value) })}
                />
              </label>
              <div className="field">
                <span>Image analysis</span>
                <div className="muted small" style={{ paddingTop: 6 }}>
                  Always runs with thinking off. It describes what it sees, so reasoning adds cost without a gain. Set thinking for the
                  language document on its provider.
                </div>
              </div>
              <label className="field">
                <span>Monthly token budget (millions, 0 = no limit)</span>
                <input
                  type="number"
                  min={0}
                  step={1}
                  value={settings.monthly_token_budget / 1_000_000}
                  onChange={(e) => setSettings({ ...settings, monthly_token_budget: Math.round(Number(e.target.value) * 1_000_000) })}
                />
              </label>
            </div>
            {status && (
              <p className="muted small">
                This month ({status.usage.month}): {fmtTokens(status.usage.input_tokens)} input + {fmtTokens(status.usage.output_tokens)}{' '}
                output tokens in {status.usage.requests} requests. The queue stops when it reaches the budget.
              </p>
            )}
            {pipelineDirty && (
              <div className="row">
                <button className="btn small" onClick={() => setSettings(saved)}>
                  Discard
                </button>
                <button className="btn primary small" onClick={saveSettings}>
                  Save pipeline
                </button>
              </div>
            )}
          </section>
        )}

        <CapturePanel />

        <BridgePanel />

        <StoragePanel info={info} onInfo={onInfo} onChanged={onChanged} />
      </div>
    </div>
  );
}
