import { hapticTrigger } from 'ios-haptics';
export function buttonHaptics(element: HTMLElement) {
  if (!(element instanceof HTMLButtonElement)) return;
  hapticTrigger(element);
  const label = element.querySelector<HTMLLabelElement>('[data-haptic-trigger]');
  if (!label) return;
  const input = label.querySelector('input');
  if (input) input.tabIndex = -1;
  const forward = (event: Event) => {
    event.stopPropagation();
    setTimeout(() => {
      if (!element.disabled) element.click();
    }, 0);
  };
  label.addEventListener('click', forward);
  return () => {
    label.removeEventListener('click', forward);
    label.remove();
  };
}
export function vibrate() {
  if (typeof navigator.vibrate === 'function')
    try {
      navigator.vibrate(8);
    } catch {
      /* Optional platform feedback. */
    }
}
