import { useEffect, useLayoutEffect, useRef } from 'react';

/**
 * A confirm row that takes the place of its trigger button ("Delete…" -> "Delete" + "Cancel").
 * While the row is open, Escape cancels it. When it closes, focus goes back to the trigger.
 * Put the returned ref on the trigger, and `autoFocus` on the Cancel or Keep button of the row.
 */
export function useConfirmRow<T extends HTMLElement = HTMLButtonElement>(open: boolean, cancel: () => void) {
  const trigger = useRef<T>(null);
  const cancelRef = useRef(cancel);
  const wasOpen = useRef(false);

  useLayoutEffect(() => {
    cancelRef.current = cancel;
  });

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      e.preventDefault();
      cancelRef.current();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open]);

  useEffect(() => {
    if (open) {
      wasOpen.current = true;
      return;
    }
    if (!wasOpen.current) return;
    wasOpen.current = false;
    // Only when focus went with the row; do not take it from a field the user moved to.
    const active = document.activeElement;
    if (!active || active === document.body) trigger.current?.focus();
  }, [open]);

  return trigger;
}
