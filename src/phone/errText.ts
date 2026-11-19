/** The text of a thrown value. The desktop's errText lives in api.ts, which imports Tauri. */
export function errText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
