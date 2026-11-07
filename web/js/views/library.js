import { $, el } from '../util/dom.js';
import { endpoint, protocolLabel } from '../util/protocols.js';
import { setDrawerOpen } from './drawer.js';

let connections = [];
let active = { id: null, dot: 'idle', text: '' };
const recentUsed = new Map();
let sequence = 0;
let handlers = {};

function matches(connection, query) {
  if (!query) return true;
  const haystack = [
    connection.name,
    connection.params.hostname,
    connection.params.username || '',
    connection.protocol,
    protocolLabel(connection.protocol),
  ]
    .join(' ')
    .toLowerCase();
  return haystack.includes(query);
}

function sorted(list) {
  return [...list].sort((a, b) => {
    const ra = recentUsed.get(a.id) || 0;
    const rb = recentUsed.get(b.id) || 0;
    if (ra !== rb) return rb - ra;
    return a.name.localeCompare(b.name, 'zh-Hant');
  });
}

function card(connection) {
  // The open button's visible text is just the name (its accessible name "連線 <name>" contains it).
  // The badge and endpoint sit beside it; CSS stretches the button over the whole card.
  const isCurrent = connection.id === active.id;
  const statusBadge =
    isCurrent && active.text
      ? el(
          'span',
          { class: 'card__status' },
          el('span', { class: 'dot', 'data-state': active.dot, 'aria-hidden': 'true' }),
          el('span', {}, active.text),
        )
      : null;
  return el(
    'article',
    { class: 'card', 'data-active': isCurrent ? 'true' : null },
    el(
      'div',
      { class: 'card__title' },
      el('span', { class: 'badge' }, protocolLabel(connection.protocol)),
      el(
        'button',
        {
          class: 'card__open',
          type: 'button',
          'aria-label': '連線 ' + connection.name,
          onclick: () => {
            recentUsed.set(connection.id, ++sequence);
            handlers.onOpen(connection);
          },
        },
        connection.name,
      ),
      statusBadge,
    ),
    el('p', { class: 'card__meta' }, endpoint(connection)),
    el(
      'div',
      { class: 'card__actions' },
      el(
        'button',
        { class: 'btn btn--quiet btn--small', type: 'button', 'aria-label': '編輯 ' + connection.name, onclick: () => handlers.onEdit(connection) },
        '編輯',
      ),
      el(
        'button',
        { class: 'btn btn--quiet btn--small', type: 'button', 'aria-label': '刪除 ' + connection.name, onclick: () => handlers.onDelete(connection) },
        '刪除',
      ),
    ),
  );
}

export function renderLibrary() {
  const query = $('library-search').value.trim().toLowerCase();
  const visible = sorted(connections).filter(connection => matches(connection, query));
  $('connections-list').replaceChildren(...visible.map(card));
  $('no-match').hidden = visible.length > 0 || connections.length === 0;
  return visible;
}

export function setConnections(list) {
  connections = list;
  renderLibrary();
}

export function setActive(id, dot = 'idle', text = '') {
  if (id) recentUsed.set(id, ++sequence);
  active = { id, dot, text };
  renderLibrary();
}

export function initLibrary(callbacks) {
  handlers = callbacks;
  $('add-connection').addEventListener('click', () => handlers.onAdd());
  const search = $('library-search');
  search.addEventListener('input', renderLibrary);
  search.addEventListener('keydown', event => {
    if (event.key === 'Enter') {
      const visible = renderLibrary();
      if (visible.length > 0) {
        event.preventDefault();
        recentUsed.set(visible[0].id, ++sequence);
        handlers.onOpen(visible[0]);
      }
    }
  });
  document.addEventListener('keydown', event => {
    if (event.key !== '/' || event.ctrlKey || event.metaKey || event.altKey) return;
    if ($('workspace').hidden || document.querySelector('dialog[open]')) return;
    const tag = document.activeElement?.tagName;
    if (['INPUT', 'TEXTAREA', 'SELECT'].includes(tag) || document.activeElement === $('terminal')) return;
    event.preventDefault();
    setDrawerOpen(true);
    search.focus();
  });
}
