import { invoke } from '@tauri-apps/api/core';

export interface AppInfo {
  library_path: string | null;
  default_library: string;
  config_path: string;
  mcp_command: string | null;
  version: string;
  /** Other libraries that the app opened before. */
  recent_libraries: string[];
  /** Set when the library folder syncs with a cloud service. */
  library_warning: string | null;
}

export interface FolderInfo {
  path: string;
  exists: boolean;
  is_library: boolean;
  empty: boolean;
  warning: string | null;
}

export interface MoveResult {
  report: { new_path: string; files: number; bytes: number };
  info: AppInfo;
  old_path: string;
  trash_error: string | null;
  trashed: boolean;
}

export interface MoveProgress {
  done: number;
  total: number;
}

export type ApiFormat = 'openai' | 'anthropic';

/** A ready-made provider. */
export interface Preset {
  kind: string;
  name: string;
  api: ApiFormat;
  base_url: string;
  model: string;
  needs_key: boolean;
  local: boolean;
  max_output: number;
  key_url: string;
  note: string;
}

/** One provider plus one model, with its own key file. */
export interface ProviderProfile {
  id: string;
  name: string;
  kind: string;
  api: ApiFormat;
  base_url: string;
  model: string;
  thinking: 'off' | 'low' | 'high';
  max_output: number;
  vision: boolean | null;
  key_account: string;
  needs_key: boolean;
}

export interface AiConfig {
  profiles: ProviderProfile[];
  analysis_profile: string;
  synthesis_profile: string;
}

export interface AiView {
  config: AiConfig;
  presets: Preset[];
  keys: Record<string, boolean>;
}

export interface CheckReport {
  message: string;
  vision: boolean;
}

export interface Language {
  id: string;
  name: string;
  description: string;
  created_at: string;
  updated_at: string;
  item_count: number;
  latest_version: number | null;
  /** Links and documents. */
  source_count: number;
}

export interface Item {
  id: string;
  language_id: string;
  kind: string;
  file_rel: string;
  thumb_rel: string;
  original_name: string;
  source: string | null;
  width: number;
  height: number;
  added_at: string;
  tags: string[];
  note: string;
  weight: number;
  anti: boolean;
  analysis_status: 'pending' | 'analyzing' | 'done' | 'error' | string;
  analysis_error: string | null;
  region_count: number;
}

export interface Region {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  note: string;
  tag: string;
}

export interface RegionInput {
  id: string | null;
  x: number;
  y: number;
  w: number;
  h: number;
  note: string;
  tag: string;
}

export interface ItemDetail {
  item: Item;
  regions: Region[];
  analysis: Record<string, unknown> | null;
  analysis_model: string | null;
}

export interface VersionInfo {
  language_id: string;
  version: number;
  created_at: string;
  model: string;
  item_count: number;
  edited: boolean;
  /** 1: free markdown. 2: structured spec; the app renders the markdown. */
  schema: number;
  /** Findings of the automatic review. */
  warnings: string[];
}

export interface VersionContent {
  info: VersionInfo;
  markdown: string;
  tokens: unknown;
}

export type PinRole = 'primary' | 'secondary';
export type PinScope = 'all' | 'cad' | 'cmf' | 'prompts';

export interface Pin {
  language_id: string;
  language_name: string;
  version: number;
  role: PinRole;
  scope: PinScope;
  latest_version: number | null;
}

export interface PinInput {
  language_id: string;
  version: number;
  role: PinRole;
  scope: PinScope;
}

export interface Project {
  id: string;
  name: string;
  created_at: string;
  pins: Pin[];
  /** Rule sets that this project selects. Sets marked "always" apply too. */
  rule_set_ids: string[];
}

export type RuleUnit = 'mm' | 'deg' | 'percent' | 'count' | 'ratio';

export interface Rule {
  name: string;
  value: number | null;
  unit: RuleUnit | null;
  note: string;
}

export interface RuleSet {
  id: string;
  name: string;
  description: string;
  always: boolean;
  rules: Rule[];
  created_at: string;
  updated_at: string;
  project_count: number;
}

export interface RuleSetInput {
  name: string;
  description: string;
  always: boolean;
  rules: Rule[];
}

export interface Settings {
  base_url: string;
  model: string;
  debounce_secs: number;
  monthly_token_budget: number;
  /** Thinking mode of the synthesis pass. */
  thinking: 'off' | 'low' | 'high';
}

export interface Usage {
  month: string;
  input_tokens: number;
  output_tokens: number;
  requests: number;
}

export interface LanguageStatus {
  language_id: string;
  pending_items: number;
  error_items: number;
  synth_due_in: number | null;
}

export interface CurrentJob {
  job_id: number;
  kind: 'analyze_item' | 'analyze_source' | 'synthesize' | string;
  language_id: string;
  language_name: string;
  item_id: string | null;
  item_name: string | null;
  title: string;
  phase: string;
  attempt: number;
  max_attempts: number;
  started_ms: number;
  phase_ms: number;
  /** '', 'connecting', 'queued', 'thinking', 'writing', 'receiving' */
  stream: string;
  call_started_ms: number | null;
  last_data_ms: number | null;
  reasoning_chars: number;
  output_chars: number;
  max_tokens: number;
  cancel_requested: boolean;
}

export interface LogEntry {
  at_ms: number;
  level: 'info' | 'ok' | 'warn' | 'error' | string;
  language_id: string | null;
  text: string;
}

export interface JobView {
  id: number;
  kind: 'analyze_item' | 'analyze_source' | 'synthesize' | string;
  target: string;
  language_id: string;
  language_name: string;
  item_name: string | null;
  run_after_ms: number;
  attempts: number;
  last_error: string | null;
  running: boolean;
}

export interface ActivityReport {
  current: CurrentJob | null;
  jobs: JobView[];
  log: LogEntry[];
  paused: boolean;
  manual: boolean;
  running: boolean;
  now_ms: number;
}

export interface StatusReport {
  worker: { state: 'idle' | 'working' | 'paused' | string; message: string; last_event: string };
  languages: LanguageStatus[];
  usage: Usage;
  budget: number;
  has_key: boolean;
  current: CurrentJob | null;
  paused: boolean;
  manual: boolean;
  running: boolean;
  queued: number;
  now_ms: number;
}

export type SourceMedia = 'page' | 'video' | 'pdf' | 'text' | 'doc';

/** A link or a document that the analysis reads, for example a tutorial. */
export interface Source {
  id: string;
  language_id: string;
  kind: 'link' | 'document';
  title: string;
  url: string | null;
  site: string;
  description: string;
  media: SourceMedia | string;
  file_rel: string | null;
  preview_rel: string | null;
  original_name: string;
  text_chars: number;
  word_count: number;
  added_at: string;
  note: string;
  weight: number;
  analysis_status: 'pending' | 'analyzing' | 'done' | 'error' | string;
  analysis_error: string | null;
}

export interface SourceDetail {
  source: Source;
  analysis: Record<string, unknown> | null;
  analysis_model: string | null;
  text_preview: string;
}

export interface SourceImportReport {
  added: Source[];
  errors: string[];
}

/** File types that the app reads as documents. */
export const DOC_EXTENSIONS = ['pdf', 'txt', 'md', 'markdown', 'csv', 'html', 'htm', 'doc', 'docx', 'rtf', 'odt'];
export const DOC_EXT_RE = new RegExp(`\\.(${DOC_EXTENSIONS.join('|')})$`, 'i');
export const IMAGE_URL_RE = /\.(png|jpe?g|webp|gif|bmp|tiff?|avif)(\?.*)?$/i;

export interface ImportReport {
  added: Item[];
  errors: string[];
}

export const api = {
  appInfo: () => invoke<AppInfo>('app_info'),
  setLibrary: (path: string) => invoke<AppInfo>('set_library', { path }),

  listLanguages: () => invoke<Language[]>('list_languages'),
  createLanguage: (name: string, description: string) =>
    invoke<Language>('create_language', { name, description }),
  updateLanguage: (id: string, name: string, description: string) =>
    invoke<Language>('update_language', { id, name, description }),

  listItems: (languageId: string) => invoke<Item[]>('list_items', { languageId }),
  itemDetail: (itemId: string) => invoke<ItemDetail>('item_detail', { itemId }),
  importFiles: (languageId: string, paths: string[]) =>
    invoke<ImportReport>('import_files', { languageId, paths }),
  importBytes: (languageId: string, name: string, data: string, source: string | null) =>
    invoke<Item>('import_bytes', { languageId, name, data, source }),
  importUrl: (languageId: string, url: string, source: string | null) =>
    invoke<Item>('import_url', { languageId, url, source }),
  updateItem: (itemId: string, tags: string[], note: string, weight: number, anti: boolean) =>
    invoke<Item>('update_item', { itemId, tags, note, weight, anti }),
  setRegions: (itemId: string, regions: RegionInput[]) =>
    invoke<Region[]>('set_regions', { itemId, regions }),
  removeItem: (itemId: string) => invoke<void>('remove_item', { itemId }),
  reanalyzeItem: (itemId: string) => invoke<void>('reanalyze_item', { itemId }),
  listSources: (languageId: string) => invoke<Source[]>('list_sources', { languageId }),
  sourceDetail: (sourceId: string) => invoke<SourceDetail>('source_detail', { sourceId }),
  addLink: (languageId: string, url: string) => invoke<Source>('add_link', { languageId, url }),
  importSourceFiles: (languageId: string, paths: string[]) =>
    invoke<SourceImportReport>('import_source_files', { languageId, paths }),
  importSourceBytes: (languageId: string, name: string, data: string) =>
    invoke<Source>('import_source_bytes', { languageId, name, data }),
  updateSource: (sourceId: string, title: string, note: string, weight: number) =>
    invoke<Source>('update_source', { sourceId, title, note, weight }),
  removeSource: (sourceId: string) => invoke<void>('remove_source', { sourceId }),
  reanalyzeSource: (sourceId: string) => invoke<void>('reanalyze_source', { sourceId }),
  openSource: (sourceId: string) => invoke<void>('open_source', { sourceId }),
  retryFailed: (languageId: string) => invoke<number>('retry_failed', { languageId }),
  reanalyzeAll: (languageId: string) => invoke<number>('reanalyze_all', { languageId }),

  listVersions: (languageId: string) => invoke<VersionInfo[]>('list_versions', { languageId }),
  getVersion: (languageId: string, version: number) =>
    invoke<VersionContent>('get_version', { languageId, version }),
  saveEditedVersion: (languageId: string, markdown: string, tokens: string) =>
    invoke<number>('save_edited_version', { languageId, markdown, tokens }),
  synthesizeNow: (languageId: string) => invoke<void>('synthesize_now', { languageId }),
  languageText: (languageId: string, version: number | null) =>
    invoke<string>('language_text', { languageId, version }),

  listProjects: () => invoke<Project[]>('list_projects'),
  createProject: (name: string) => invoke<Project>('create_project', { name }),
  renameProject: (id: string, name: string) => invoke<Project>('rename_project', { id, name }),
  deleteProject: (id: string) => invoke<void>('delete_project', { id }),
  setPins: (projectId: string, pins: PinInput[]) => invoke<Project>('set_pins', { projectId, pins }),
  projectBrief: (projectId: string) => invoke<string>('project_brief', { projectId }),
  setProjectRuleSets: (projectId: string, ruleSetIds: string[]) =>
    invoke<Project>('set_project_rule_sets', { projectId, ruleSetIds }),

  listRuleSets: () => invoke<RuleSet[]>('list_rule_sets'),
  createRuleSet: (input: RuleSetInput) => invoke<RuleSet>('create_rule_set', { input }),
  updateRuleSet: (id: string, input: RuleSetInput) => invoke<RuleSet>('update_rule_set', { id, input }),
  deleteRuleSet: (id: string) => invoke<void>('delete_rule_set', { id }),
  ruleSetText: (id: string) => invoke<string>('rule_set_text', { id }),

  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  getAi: () => invoke<AiView>('get_ai'),
  saveAi: (config: AiConfig) => invoke<AiView>('save_ai', { config }),
  newProfile: (kind: string) => invoke<ProviderProfile>('new_profile', { kind }),
  setProviderKey: (profileId: string, key: string) => invoke<void>('set_provider_key', { profileId, key }),
  clearProviderKey: (profileId: string) => invoke<void>('clear_provider_key', { profileId }),
  checkProvider: (profile: ProviderProfile) => invoke<CheckReport>('check_provider', { profile }),
  listModels: (profile: ProviderProfile) => invoke<string[]>('list_models', { profile }),
  folderInfo: (path: string) => invoke<FolderInfo>('folder_info', { path }),
  revealPath: (path: string) => invoke<void>('reveal_path', { path }),
  moveLibrary: (path: string, trashOld: boolean) => invoke<MoveResult>('move_library', { path, trashOld }),
  status: () => invoke<StatusReport>('status'),
  activity: () => invoke<ActivityReport>('activity'),
  cancelJob: (jobId: number) => invoke<void>('cancel_job', { jobId }),
  cancelAll: (languageId: string | null) => invoke<number>('cancel_all', { languageId }),
  setPaused: (paused: boolean) => invoke<void>('set_paused', { paused }),
  setManual: (manual: boolean) => invoke<void>('set_manual', { manual }),
  runQueue: (run: boolean) => invoke<void>('run_queue', { run }),
};

/** URL of a file in the library, served by the app's cory:// protocol. */
export function fileUrl(rel: string): string {
  return 'cory://localhost/' + rel.split('/').map(encodeURIComponent).join('/');
}

export function shortId(id: string): string {
  return id.slice(0, 8);
}

export function errText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

export function fmtDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

export function fmtTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + ' M';
  if (n >= 1_000) return (n / 1_000).toFixed(1) + ' k';
  return String(n);
}

/** Reads a File as base64 (no data: prefix). */
export function fileToBase64(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => {
      const s = String(r.result);
      resolve(s.slice(s.indexOf(',') + 1));
    };
    r.onerror = () => reject(r.error);
    r.readAsDataURL(file);
  });
}

/** "2:05" or "45 s". */
export function fmtDuration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}:${String(s % 60).padStart(2, '0')}`;
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, '0')} min`;
}

/** Plain words for the stream state of the current model call. */
export function streamLabel(c: CurrentJob): string {
  switch (c.stream) {
    case 'connecting':
      return 'Sending the request';
    case 'queued':
      return 'Waiting in the provider\'s queue (the server is busy)';
    case 'thinking':
      return 'The model is thinking';
    case 'writing':
      return 'The model is writing its answer';
    case 'receiving':
      return 'Receiving the answer';
    default:
      return c.phase;
  }
}

/** Short status for a chip or the sidebar. */
export function shortStatus(c: CurrentJob, nowMs: number): string {
  const t = fmtDuration(nowMs - c.started_ms);
  if (c.cancel_requested) return `Stopping · ${t}`;
  const verb = c.kind === 'synthesize' ? 'Writing' : c.kind === 'analyze_source' ? 'Reading' : 'Analyzing';
  const what = c.stream === 'thinking' ? 'thinking' : c.stream === 'writing' ? 'writing' : c.stream === 'queued' ? 'in provider queue' : c.phase.toLowerCase();
  const attempt = c.kind === 'synthesize' && c.attempt > 1 ? ` · attempt ${c.attempt}/${c.max_attempts}` : '';
  return `${verb} · ${what}${attempt} · ${t}`;
}

/** "1 240 words" or "12 k words". */
export function fmtWords(n: number): string {
  if (n <= 0) return 'no text';
  if (n >= 10_000) return `${Math.round(n / 1000)} k words`;
  return `${n.toLocaleString()} word${n === 1 ? '' : 's'}`;
}

export function mediaLabel(m: string): string {
  switch (m) {
    case 'page':
      return 'Web page';
    case 'video':
      return 'Video';
    case 'pdf':
      return 'PDF';
    case 'text':
      return 'Text';
    case 'doc':
      return 'Document';
    default:
      return m;
  }
}

/** "1.2 GB", "340 MB", "12 kB". */
export function fmtBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} kB`;
  return `${n} B`;
}
