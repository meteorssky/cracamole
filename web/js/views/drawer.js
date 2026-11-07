import { $ } from '../util/dom.js';

// The connection library: always visible when idle, tucked away during a session to give the terminal the space.
export function setDrawerOpen(open) {
  $('library').hidden = !open;
  const button = $('toggle-library');
  button.setAttribute('aria-expanded', String(open));
  button.setAttribute('aria-label', open ? '收合連線庫' : '開啟連線庫');
}

export function isDrawerOpen() {
  return !$('library').hidden;
}

export function initDrawer() {
  $('toggle-library').addEventListener('click', () => setDrawerOpen(!isDrawerOpen()));
}
