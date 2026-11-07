import { $ } from '../util/dom.js';

let timer = null;

// Supplementary feedback only: errors must also be shown where they happened.
export function toast(text) {
  const node = $('toast');
  node.textContent = text;
  node.hidden = false;
  clearTimeout(timer);
  timer = setTimeout(() => {
    node.hidden = true;
  }, 3500);
}
