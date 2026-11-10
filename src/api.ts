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
  /** US dollars for one million input tokens; 0 means not set. */
  price_in: number;
  /** US dollars for one million output tokens. */
  price_out: number;
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
  /** The language that this one extends. Its document is the base plus the overrides. */
  base_id: string | null;
  /** The base version to follow; null follows the latest. */
  base_version: number | null;
  override_count: number;
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
  /** Place on the canvas view; null until placed. */
  canvas: CanvasPos | null;
  /** The 3D model behind a rendered item (kind "model"). */
  model_rel: string | null;
  /** The main colours of the image, largest share first. */
  palette: Swatch[];
}

/** One colour of a palette and the share that it covers (0 to 1). */
export interface Swatch {
  hex: string;
  share: number;
}

/** Canvas units: 1 unit = 1 px at 100 %. The height follows from the image proportions. */
export interface CanvasPos {
  x: number;
  y: number;
  w: number;
}

export interface CanvasPlacement extends CanvasPos {
  id: string;
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
  /** Amendment proposals from agents that wait for a decision. */
  open_proposals: number;
  /** Agent outputs that wait for a verdict. */
  pending_outputs: number;
}

export interface Measurement {
  name: string;
  value: number;
  unit: string | null;
}

export interface ConformanceFinding {
  measure: string;
  value: number;
  unit: string | null;
  verdict: 'pass' | 'fail' | 'warn' | 'unknown';
  source: string;
  expected: string;
  detail: string;
}

export interface ConformanceReport {
  language: string;
  version: number;
  family: string | null;
  findings: ConformanceFinding[];
  not_reported: string[];
  /** Rule sets that were applied; empty means no hard rule judged anything. */
  rule_sets: string[];
  /** Entries of the print log about a material. */
  print_log?: string[];
  pass: number;
  fail: number;
  warn: number;
  unknown: number;
}

/** A measured value and where on the model it is. */
export interface Spot {
  value: number;
  at: [number, number, number];
}

/** The geometry check of a model file. */
export interface GeometryReport {
  file: string;
  format: string;
  up: string;
  triangles: number;
  bodies: number;
  cavities: number;
  open_edges: number;
  size: [number, number, number];
  volume_cm3: number | null;
  area_cm2: number;
  min_wall: Spot | null;
  thinnest: Spot | null;
  median_wall: number | null;
  max_overhang: Spot | null;
  overhang_area_mm2: number;
  overhang_limit: number;
  max_bridge: Spot | null;
  flat_overhang: Spot | null;
  min_clearance: Spot | null;
  overlap: boolean;
  contact_mm2: number;
  measurements: Measurement[];
  notes: string[];
  check: ConformanceReport | null;
  seconds: number;
}

/** A design that an agent made from a language and sent back for a verdict. */
export interface Output {
  id: string;
  language_id: string;
  project: string;
  name: string;
  file_rel: string;
  thumb_rel: string;
  width: number;
  height: number;
  language_version: number | null;
  author: string;
  note: string;
  family: string;
  measurements: Measurement[];
  check: ConformanceReport | null;
  /** The geometry check of a model of this design, if one ran. */
  geometry: GeometryReport | null;
  verdict: 'pending' | 'accepted' | 'rejected';
  verdict_note: string;
  item_id: string | null;
  created_at: string;
  decided_at: string | null;
}

export interface ProposalChange {
  op: 'add' | 'remove' | 'replace';
  path: string;
  before: unknown;
  after: unknown;
}

/** A change to a language spec that an agent proposed through the MCP server. */
export interface Proposal {
  id: string;
  language_id: string;
  base_version: number;
  summary: string;
  rationale: string;
  author: string;
  status: 'pending' | 'accepted' | 'rejected';
  created_at: string;
  decided_at: string | null;
  result_version: number | null;
  /** The changes against the latest version (pending proposals only). */
  changes: ProposalChange[];
  /** Why the proposal cannot be accepted now. */
  problem: string | null;  /** Review notes that accepting would add. */
  new_notes: string[];
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

export interface VideoNote {
  name: string;
  frames: number;
  /** Length of the video in seconds. */
  duration: number;
}

export interface ImportReport {
  added: Item[];
  errors: string[];
  /** One entry for each video: its key frames became items. */
  videos: VideoNote[];
}

export interface VideoImport {
  items: Item[];
  duration: number;
}

/** Videos that macOS (AVFoundation) reads; their key frames become items. */
export const VIDEO_EXTENSIONS = ['mp4', 'mov', 'm4v'];
export const VIDEO_EXT_RE = /\.(mp4|mov|m4v|webm|avi|mkv|flv|wmv)$/i;


/** Screen capture from the menu bar and the keyboard shortcut. */
export interface CaptureView {
  /** The language that captures go to now. */
  language_id: string | null;
  language_name: string | null;
  /** "Control+Alt+Super+KeyC", or null for no shortcut. */
  shortcut: string | null;
  /** Why the shortcut does not work, for example another app has it. */
  shortcut_error: string | null;
}

export interface CaptureDone {
  language_id: string;
  language_name: string;
  item_id: string;
}

const KEY_NAMES: Record<string, string> = {
  Space: 'Space', Enter: '↩', Tab: '⇥', Backspace: '⌫', Escape: '⎋', Minus: '-', Equal: '=', BracketLeft: '[',
  BracketRight: ']', Backslash: '\\', Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/', Backquote: '`',
  ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→',
};

/** "⌃⌥⌘C" for "Control+Alt+Super+KeyC". */
export function shortcutLabel(s: string): string {
  const parts = s.split('+');
  const key = parts.pop() ?? '';
  const mods = new Set(parts.map((p) => p.toLowerCase()));
  let out = '';
  if (mods.has('control') || mods.has('ctrl')) out += '⌃';
  if (mods.has('alt') || mods.has('option')) out += '⌥';
  if (mods.has('shift')) out += '⇧';
  if (mods.has('super') || mods.has('cmd') || mods.has('command') || mods.has('meta')) out += '⌘';
  const k = key.replace(/^Key/, '').replace(/^Digit/, '');
  return out + (KEY_NAMES[key] ?? k);
}

/** The shortcut string of a key press, or null for a modifier alone or a press without a modifier. */
export function shortcutFromEvent(e: KeyboardEvent): string | null {
  if (['ShiftLeft', 'ShiftRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight'].includes(e.code)) return null;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push('Control');
  if (e.altKey) mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (e.metaKey) mods.push('Super');
  // Function keys work alone; other keys need a modifier other than Shift.
  const isF = /^F\d{1,2}$/.test(e.code);
  if (!isF && mods.filter((m) => m !== 'Shift').length === 0) return null;
  return [...mods, e.code].join('+');
}

// ---------------------------------------------------------------- Phase 3.2 to 3.4

export type PrintOutcome = 'failed' | 'warped' | 'frosted' | 'fine' | 'other';
export const PRINT_OUTCOMES: PrintOutcome[] = ['failed', 'warped', 'frosted', 'fine', 'other'];

/** What happened when a design was printed. */
export interface PrintResult {
  id: string;
  result: PrintOutcome;
  material: string;
  /** The hard rule that the result is about, or empty. */
  rule: string;
  /** null: the entry applies everywhere. */
  rule_set_id: string | null;
  rule_set_name: string | null;
  value: number | null;
  unit: string | null;
  note: string;
  language_id: string | null;
  output_id: string | null;
  /** Empty for you; else the agent that wrote the entry. */
  author: string;
  created_at: string;
}

export interface PrintResultInput {
  result: PrintOutcome;
  material: string;
  rule: string;
  rule_set: string | null;
  value: number | null;
  unit: string | null;
  note: string;
  language: string | null;
  output: string | null;
}

export interface RuleFeedback {
  rule: string;
  failed: number;
  warped: number;
  frosted: number;
  fine: number;
  other: number;
  hints: string[];
}

export interface ProfileRef {
  app: string;
  kind: 'process' | 'machine' | 'filament';
  name: string;
  path: string;
  user: boolean;
}

export interface SlicerInput {
  process: string | null;
  machine: string | null;
  filament: string | null;
  project: string | null;
}

export interface CompareRow {
  rule: string;
  unit: string | null;
  imported: number | null;
  builtin: number | null;
  status: 'same' | 'differs' | 'new' | 'builtin-only';
}

export interface SlicerDerived {
  name: string;
  description: string;
  rules: Rule[];
  compare: CompareRow[];
  notes: string[];
}

export interface DiffChange {
  section: string;
  kind: 'added' | 'removed' | 'changed';
  subject: string;
  text: string;
}

export interface VersionDiff {
  language: string;
  from: number;
  to: number;
  /** false when a version uses the older format: only the text can be compared. */
  structured: boolean;
  changes: DiffChange[];
}

export interface Gap {
  kind: string;
  severity: 'high' | 'medium' | 'low';
  family: string | null;
  title: string;
  detail: string;
  suggestion: string;
  evidence: string[];
}

export interface FamilyCoverage {
  id: string;
  name: string;
  references: number;
  anti_references: number;
  measures: number;
  rules: number;
  views: string[];
  missing_views: string[];
}

export interface Coverage {
  language: string;
  version: number | null;
  families: FamilyCoverage[];
  gaps: Gap[];
  unused_references: number;
}

export interface OverrideView {
  op: string;
  path: string;
  before: unknown;
  after: unknown;
  problem: string | null;
}

export interface Inheritance {
  base_id: string;
  base_name: string;
  pinned_version: number | null;
  built_from: number | null;
  base_latest: number | null;
  overrides: OverrideView[];
  error: string | null;
}

export type ExportFormat = 'css' | 'figma' | 'grasshopper';
export const EXPORT_FORMATS: { id: ExportFormat; label: string }[] = [
  { id: 'css', label: 'CSS variables' },
  { id: 'figma', label: 'Figma tokens (JSON)' },
  { id: 'grasshopper', label: 'Grasshopper parameters (JSON)' },
];

export interface Exported {
  format: string;
  file_name: string;
  content: string;
}

export interface Pass {
  calls: number;
  input_tokens: number;
  output_tokens: number;
}

export interface Estimate {
  images: Pass;
  sources: Pass;
  document: Pass;
  total_tokens: number;
  /** US dollars; null when a provider has no prices. */
  cost_usd: number | null;
  from_history: boolean;
  budget_left: number | null;
}

export type EstimateScope = 'queue' | 'all' | 'document' | 'other-model';

export interface OtherModelView {
  /** The model that analyzes images now. */
  model: string;
  other: { images: number; sources: number; models: string[] };
}

export interface SnapshotReport {
  path: string;
  files: number;
  bytes: number;
}

export interface DbCopy {
  name: string;
  path: string;
  bytes: number;
}

export interface FamilyUse {
  id: string;
  name: string;
  fetches: number;
  checks: number;
  last_used: string | null;
}

export interface Recurring {
  path: string;
  proposals: number;
  pending: number;
  accepted: number;
  rejected: number;
  summaries: string[];
}

export interface UsageInsight {
  language: string;
  since: string;
  days: number;
  calls: number;
  document_reads: number;
  briefs: number;
  last_call: string | null;
  tools: [string, number][];
  clients: [string, number][];
  families: FamilyUse[];
  unused_families: string[];
  measures: { family: string; name: string; checks: number }[];
  unchecked_measures: string[];
  references: [string, number][];
  recurring: Recurring[];
}

export interface BoardItem {
  image_url: string;
  fallback_url: string | null;
  title: string;
  source: string;
}

export interface BoardPreview {
  service: string;
  title: string;
  url: string;
  items: BoardItem[];
  skipped: number;
  note: string | null;
}

export interface BoardImportSummary {
  added: number;
  duplicates: number;
  errors: string[];
}

export interface BoardHit {
  item_id: string;
  matched: number;
  snippets: string[];
}

export interface BridgeView {
  enabled: boolean;
  port: number;
  token: string;
  running: boolean;
  error: string | null;
}

/** 3D model files that the app reads; it draws four views of each. */
export const MODEL_EXTENSIONS = ['stl', '3mf', 'obj', 'step', 'stp'];
export const MODEL_EXT_RE = /\.(stl|3mf|obj|step|stp)$/i;

/** "about 62 k tokens (≈ $0.04)" */
export function fmtEstimate(e: Estimate): string {
  const money = e.cost_usd === null ? '' : ` (≈ $${e.cost_usd < 0.01 && e.cost_usd > 0 ? '<0.01' : e.cost_usd.toFixed(2)})`;
  return `about ${fmtTokens(e.total_tokens)} tokens${money}`;
}

/** "1.2 mm", "45°", "0.4 ×" */
export function fmtMeasure(v: number, unit: string | null): string {
  const n = String(Math.round(v * 100) / 100);
  switch (unit) {
    case 'mm':
      return `${n} mm`;
    case 'deg':
      return `${n}°`;
    case 'percent':
      return `${n} %`;
    case 'ratio':
      return `${n} ×`;
    default:
      return n;
  }
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
  setCanvas: (languageId: string, places: CanvasPlacement[]) => invoke<number>('set_canvas', { languageId, places }),
  clearCanvas: (languageId: string) => invoke<void>('clear_canvas', { languageId }),
  itemDetail: (itemId: string) => invoke<ItemDetail>('item_detail', { itemId }),
  importFiles: (languageId: string, paths: string[]) =>
    invoke<ImportReport>('import_files', { languageId, paths }),
  /** A dropped video, sent as raw bytes (no base64). */
  importVideoBytes: async (languageId: string, file: File) =>
    invoke<VideoImport>('import_video_bytes', new Uint8Array(await file.arrayBuffer()), {
      headers: { 'x-language-id': encodeURIComponent(languageId), 'x-file-name': encodeURIComponent(file.name || 'video.mov') },
    }),
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
  listProposals: (languageId: string, includeDecided = false) =>
    invoke<Proposal[]>('list_proposals', { languageId, includeDecided }),
  acceptProposal: (proposalId: string) => invoke<number>('accept_proposal', { proposalId }),
  rejectProposal: (proposalId: string) => invoke<void>('reject_proposal', { proposalId }),
  listOutputs: (languageId: string) => invoke<Output[]>('list_outputs', { languageId }),
  setOutputVerdict: (outputId: string, verdict: Output['verdict'], note: string) =>
    invoke<Output>('set_output_verdict', { outputId, verdict, note }),
  removeOutput: (outputId: string) => invoke<void>('remove_output', { outputId }),
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
  getCapture: () => invoke<CaptureView>('get_capture'),
  setCapture: (languageId: string | null, shortcut: string | null) =>
    invoke<CaptureView>('set_capture', { languageId, shortcut }),
  captureNow: () => invoke<void>('capture_now'),
  importModelBytes: async (languageId: string, file: File) =>
    invoke<Item>('import_model_bytes', new Uint8Array(await file.arrayBuffer()), {
      headers: { 'x-language-id': encodeURIComponent(languageId), 'x-file-name': encodeURIComponent(file.name || 'model.stl') },
    }),
  backfillPalettes: (languageId: string) => invoke<number>('backfill_palettes', { languageId }),
  languagePalette: (languageId: string) => invoke<Swatch[]>('language_palette', { languageId }),
  searchBoard: (languageId: string, query: string) => invoke<BoardHit[]>('search_board', { languageId, query }),
  checkOutputGeometry: (outputId: string, path: string, up: string | null) =>
    invoke<Output>('check_output_geometry', { outputId, path, up }),
  listPrintResults: () => invoke<PrintResult[]>('list_print_results'),
  addPrintResult: (input: PrintResultInput) => invoke<PrintResult>('add_print_result', { input }),
  removePrintResult: (id: string) => invoke<void>('remove_print_result', { id }),
  ruleFeedback: (ruleSetId: string) => invoke<RuleFeedback[]>('rule_feedback', { ruleSetId }),
  slicerProfiles: () => invoke<ProfileRef[]>('slicer_profiles'),
  slicerPreview: (input: SlicerInput) => invoke<SlicerDerived>('slicer_preview', { input }),
  slicerImport: (input: SlicerInput, name: string, fillFromBuiltin: boolean) =>
    invoke<RuleSet>('slicer_import', { input, name, fillFromBuiltin }),
  versionDiff: (languageId: string, from: number, to: number) => invoke<VersionDiff>('version_diff', { languageId, from, to }),
  restoreVersion: (languageId: string, version: number) => invoke<number>('restore_version', { languageId, version }),
  coverage: (languageId: string) => invoke<Coverage>('coverage', { languageId }),
  setBase: (languageId: string, base: string | null, version: number | null) =>
    invoke<Language>('set_base', { languageId, base, version }),
  inheritance: (languageId: string) => invoke<Inheritance | null>('inheritance', { languageId }),
  removeOverride: (languageId: string, index: number) => invoke<number | null>('remove_override', { languageId, index }),
  exportLanguage: (languageId: string, version: number | null, format: ExportFormat, project: string | null) =>
    invoke<Exported>('export_language', { languageId, version, format, project }),
  saveTextFile: (path: string, content: string) => invoke<void>('save_text_file', { path, content }),
  estimate: (languageId: string, scope: EstimateScope) => invoke<Estimate>('estimate', { languageId, scope }),
  otherModel: (languageId: string) => invoke<OtherModelView>('other_model', { languageId }),
  reanalyzeOtherModel: (languageId: string) => invoke<number>('reanalyze_other_model', { languageId }),
  createSnapshot: (dest: string) => invoke<SnapshotReport>('create_snapshot', { dest }),
  restoreSnapshot: (zip: string, dest: string) => invoke<number>('restore_snapshot', { zip, dest }),
  dbCopies: () => invoke<DbCopy[]>('db_copies'),
  usageInsight: (languageId: string, days: number) => invoke<UsageInsight>('usage_insight', { languageId, days }),
  previewBoard: (url: string) => invoke<BoardPreview>('preview_board', { url }),
  importBoard: (languageId: string, url: string) => invoke<BoardImportSummary>('import_board', { languageId, url }),
  getBridge: () => invoke<BridgeView>('get_bridge'),
  setBridge: (enabled: boolean, port: number, newTokenNow: boolean) =>
    invoke<BridgeView>('set_bridge', { enabled, port, newTokenNow }),
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
