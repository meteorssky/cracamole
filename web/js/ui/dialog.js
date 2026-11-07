import { $ } from '../util/dom.js';

// Safari and Firefox on macOS do not focus a button when it is clicked, so document.activeElement
// is useless for "return focus to the opener". Remember the last activated button ourselves.
let lastActivated = null;
document.addEventListener(
  'click',
  event => {
    const button = event.target instanceof Element ? event.target.closest('button') : null;
    if (button) lastActivated = button;
  },
  true,
);

// Opens a native <dialog> and restores focus to whatever opened it when it closes.
export function openDialog(dialog, focusTarget) {
  const active = document.activeElement;
  const trigger = active && active !== document.body ? active : lastActivated;
  dialog.addEventListener('close', () => trigger?.focus?.(), { once: true });
  dialog.showModal();
  (focusTarget || dialog.querySelector('input, select, textarea, button'))?.focus();
}

// The single confirmation mechanism (window.confirm is not used anywhere).
export function confirmAction({ title, message, confirmLabel }) {
  const dialog = $('confirm-dialog');
  $('confirm-title').textContent = title;
  $('confirm-message').textContent = message;
  $('confirm-ok').textContent = confirmLabel;
  dialog.returnValue = 'cancel';
  return new Promise(resolve => {
    dialog.addEventListener('close', () => resolve(dialog.returnValue === 'ok'), { once: true });
    // Destructive actions start on the safe choice.
    openDialog(dialog, $('confirm-cancel'));
  });
}
