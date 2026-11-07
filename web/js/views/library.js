import { $, el } from '../util/dom.js';
import { endpoint, protocolLabel } from '../util/protocols.js';

let connections = [];
let activeId = null;
let handlers = {};

function matches(connection, query) {
  if (!query) return true;
  const haystack = [connection.name, connection.params.hostname, connection.protocol, protocolLabel(connection.protocol)]
    .join(' ')
    .toLowerCase();
  return haystack.includes(query);
}

function card(connection) {
  // The open button's visible text is just the name (its accessible name "連線 <name>" contains it).
  // The badge and endpoint sit beside it; CSS stretches the button over the whole card.
  return el(
    'article',
    { class: 'card', 'data-active': connection.id === activeId ? 'true' : null },
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
          onclick: () => handlers.onOpen(connection),
        },
        connection.name,
      ),
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
  const visible = connections.filter(connection => matches(connection, query));
  $('connections-list').replaceChildren(...visible.map(card));
  $('no-match').hidden = visible.length > 0 || connections.length === 0;
}

export function setConnections(list) {
  connections = list;
  renderLibrary();
}

export function setActive(id) {
  activeId = id;
  renderLibrary();
}

export function initLibrary(callbacks) {
  handlers = callbacks;
  $('add-connection').addEventListener('click', () => handlers.onAdd());
  $('library-search').addEventListener('input', renderLibrary);
}
