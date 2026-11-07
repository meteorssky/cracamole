// Theme: an explicit choice (stored, not sensitive) wins over the system preference; dark is the fallback.
import { $ } from './dom.js';

const KEY = 'cracamole-theme';
const root = document.documentElement;

function effective() {
  const explicit = root.getAttribute('data-theme');
  if (explicit) return explicit;
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

function renderButton() {
  $('theme-toggle').textContent = effective() === 'dark' ? '淺色主題' : '深色主題';
}

export function initTheme() {
  let stored = null;
  try {
    stored = localStorage.getItem(KEY);
  } catch {
    stored = null; // storage can be blocked; the system preference still applies
  }
  if (stored === 'light' || stored === 'dark') root.setAttribute('data-theme', stored);
  renderButton();
  $('theme-toggle').addEventListener('click', () => {
    const next = effective() === 'dark' ? 'light' : 'dark';
    root.setAttribute('data-theme', next);
    try {
      localStorage.setItem(KEY, next);
    } catch {
      // not persisted; applies for this page only
    }
    renderButton();
  });
}
