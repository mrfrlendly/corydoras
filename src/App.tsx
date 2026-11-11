import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { api, AppInfo, CaptureDone, errText, Language, Project, RuleSet, StatusReport } from './api';
import Sidebar from './components/Sidebar';
import LanguageView from './components/LanguageView';
import ProjectView from './components/ProjectView';
import SettingsView from './components/SettingsView';
import ActivityView from './components/ActivityView';
import RuleSetView from './components/RuleSetView';
import SlicerImportView from './components/SlicerImportView';
import Onboarding from './components/Onboarding';
import Logo from './components/Logo';
import { ToastProvider, useToast } from './components/Toast';

export type View =
  | { kind: 'home' }
  | { kind: 'language'; id: string }
  | { kind: 'project'; id: string }
  | { kind: 'ruleset'; id: string }
  | { kind: 'slicer' }
  | { kind: 'settings' }
  | { kind: 'activity' };

function Shell() {
  const toast = useToast();
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [languages, setLanguages] = useState<Language[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [ruleSets, setRuleSets] = useState<RuleSet[]>([]);
  const [status, setStatus] = useState<StatusReport | null>(null);
  const [view, setView] = useState<View>({ kind: 'home' });
  const [tick, setTick] = useState(0);
  const busy = !!status && (status.current !== null || status.queued > 0);

  const bump = useCallback(() => setTick((t) => t + 1), []);

  useEffect(() => {
    api.appInfo().then(setInfo).catch((e) => toast.error(errText(e)));
  }, [toast]);

  // Reload the lists on each change.
  useEffect(() => {
    if (!info?.library_path) return;
    Promise.all([api.listLanguages(), api.listProjects(), api.listRuleSets()])
      .then(([l, p, r]) => {
        setLanguages(l);
        setProjects(p);
        setRuleSets(r);
      })
      .catch((e) => toast.error(errText(e)));
  }, [info, tick, toast]);

  // The worker emits an event after each job.
  useEffect(() => {
    const un = listen<string>('library-changed', () => bump());
    return () => {
      un.then((f) => f());
    };
  }, [bump]);

  // Screen captures from the menu bar or the shortcut.
  useEffect(() => {
    const uns = [
      listen<CaptureDone>('capture-done', (e) =>
        toast.info(`Screen capture added to “${e.payload.language_name}”.`, {
          label: 'Show',
          run: () => setView({ kind: 'language', id: e.payload.language_id }),
        }),
      ),
      listen<string>('capture-error', (e) => toast.error(e.payload)),
      listen<{ language_id: string; language_name: string; what: string }>('bridge-added', (e) =>
        toast.info(`The browser extension added ${e.payload.what === 'source' ? 'a page to the sources of' : 'an image to'} “${e.payload.language_name}”.`, {
          label: 'Show',
          run: () => setView({ kind: 'language', id: e.payload.language_id }),
        }),
      ),
      listen('open-settings', () => setView({ kind: 'settings' })),
    ];
    return () => {
      uns.forEach((u) => u.then((f) => f()));
    };
  }, [toast]);

  // Poll the queue status.
  useEffect(() => {
    if (!info?.library_path) return;
    let alive = true;
    const load = () =>
      api
        .status()
        .then((s) => alive && setStatus(s))
        .catch(() => {});
    load();
    const t = setInterval(load, busy ? 1000 : 2500);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [info, tick, busy]);

  // Open the first language when nothing is open.
  useEffect(() => {
    if (view.kind === 'home' && languages.length > 0) setView({ kind: 'language', id: languages[0].id });
  }, [languages, view.kind]);

  // Screen captures go into the language that is open.
  const openLanguageId = view.kind === 'language' ? view.id : null;
  useEffect(() => {
    if (openLanguageId) api.followOpenLanguage(openLanguageId).catch(() => {});
  }, [openLanguageId]);

  // ⌘, opens Settings, as in every Mac app.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && !e.ctrlKey && !e.altKey && e.key === ',') {
        e.preventDefault();
        setView({ kind: 'settings' });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // One sentence about the queue for screen readers. It changes only when the queue changes
  // state (no time counter), and the first state after launch is not read out.
  const queueNow = !status
    ? null
    : status.current
      ? status.current.kind === 'synthesize'
        ? `Writing a new version of ${status.current.language_name}.`
        : `Analysis runs in ${status.current.language_name}.`
      : status.paused
        ? 'The queue is paused.'
        : status.queued > 0
          ? 'Jobs wait in the queue.'
          : 'The queue is empty.';
  const [queueSay, setQueueSay] = useState('');
  const lastQueue = useRef<string | null>(null);
  useEffect(() => {
    if (queueNow === null) return;
    if (lastQueue.current !== null && lastQueue.current !== queueNow) setQueueSay(queueNow);
    lastQueue.current = queueNow;
  }, [queueNow]);

  if (!info)
    return (
      <div className="boot">
        <Logo size={64} />
      </div>
    );
  if (!info.library_path) return <Onboarding info={info} onDone={setInfo} />;

  const lang = view.kind === 'language' ? languages.find((l) => l.id === view.id) : undefined;
  const project = view.kind === 'project' ? projects.find((p) => p.id === view.id) : undefined;
  const ruleSet = view.kind === 'ruleset' ? ruleSets.find((r) => r.id === view.id) : undefined;

  return (
    <div className="app">
      <Sidebar
        languages={languages}
        projects={projects}
        ruleSets={ruleSets}
        status={status}
        view={view}
        onView={setView}
        onChanged={bump}
      />
      <div className="sr-only" role="status" aria-live="polite">
        {queueSay}
      </div>
      <main className="main">
        {view.kind === 'language' && lang && (
          <LanguageView
            key={lang.id}
            language={lang}
            languages={languages}
            projects={projects}
            status={status?.languages.find((s) => s.language_id === lang.id) ?? null}
            worker={status?.worker ?? null}
            current={status?.current?.language_id === lang.id ? status.current : null}
            paused={status?.paused ?? false}
            tick={tick}
            onChanged={bump}
            onSettings={() => setView({ kind: 'settings' })}
            onActivity={() => setView({ kind: 'activity' })}
          />
        )}
        {view.kind === 'project' && project && (
          <ProjectView
            key={project.id}
            project={project}
            languages={languages}
            ruleSets={ruleSets}
            onOpenRuleSet={(id) => setView({ kind: 'ruleset', id })}
            tick={tick}
            onChanged={bump}
            onDeleted={() => {
              setView({ kind: 'home' });
              bump();
            }}
          />
        )}
        {view.kind === 'settings' && <SettingsView info={info} onInfo={setInfo} status={status} onChanged={bump} />}
        {view.kind === 'ruleset' && ruleSet && (
          <RuleSetView
            key={ruleSet.id}
            ruleSet={ruleSet}
            projects={projects}
            onChanged={bump}
            onDeleted={() => {
              setView({ kind: 'home' });
              bump();
            }}
            onOpenProject={(id) => setView({ kind: 'project', id })}
          />
        )}
        {view.kind === 'slicer' && (
          <SlicerImportView
            onCancel={() => setView({ kind: 'home' })}
            onCreated={(r) => {
              bump();
              setView({ kind: 'ruleset', id: r.id });
            }}
          />
        )}
        {view.kind === 'activity' && (
          <ActivityView languages={languages} onChanged={bump} onOpenLanguage={(id) => setView({ kind: 'language', id })} />
        )}
        {view.kind === 'home' && (
          <div className="empty-main" data-tauri-drag-region>
            <div className="empty-card">
              <Logo size={56} />
              <div className="eyebrow">Corydoras</div>
              <h1>Make your first design language.</h1>
              <ol className="first-steps">
                <li>
                  Add a model provider and its API key.
                  {status?.has_key ? (
                    <span className="first-done">Done.</span>
                  ) : (
                    <button className="link-btn" onClick={() => setView({ kind: 'settings' })}>
                      Open Settings
                    </button>
                  )}
                </li>
                <li>Click + next to Languages, type a name, and press Return.</li>
                <li>Drag images into the board. The first language document follows the analysis.</li>
                <li>
                  Connect an agent: Settings › Agents (MCP server).
                  <button className="link-btn" onClick={() => setView({ kind: 'settings' })}>
                    Open Settings
                  </button>
                </li>
              </ol>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}
