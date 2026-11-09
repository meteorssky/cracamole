import { $, el } from '../util/dom.js';
import { defaultPort, endpoint, protocolLabel, quickAddress } from '../util/protocols.js';
import { setDrawerOpen } from './drawer.js';

let connections = [];
let active = { id: null, dot: 'idle', text: '' };
const recentUsed = new Map();
const statuses = new Map();
let pinned = new Set();
let collapsed = new Set();
let sequence = 0;
let handlers = {};

function readSet(key) {
  try {
    const value = JSON.parse(localStorage.getItem(key) || '[]');
    return new Set(Array.isArray(value) ? value.filter(item => typeof item === 'string') : []);
  } catch {
    return new Set();
  }
}

function saveSet(key, values) {
  try {
    localStorage.setItem(key, JSON.stringify([...values]));
  } catch {
    // 功能仍可在目前分頁使用，瀏覽器可能禁止本機儲存。
  }
}

function rememberRecent(id) {
  recentUsed.delete(id);
  recentUsed.set(id, ++sequence);
  while (recentUsed.size > 50) recentUsed.delete(recentUsed.keys().next().value);
  saveSet('cracamole-recent', recentUsed.keys());
}

function groupName(connection) {
  const slash = connection.name.indexOf('/');
  return slash > 0 ? connection.name.slice(0, slash).trim() || '未分類' : '未分類';
}

function exactConnection(target) {
  return connections.find(connection =>
    connection.protocol === target.protocol &&
    (connection.params.hostname || '').toLowerCase() === target.hostname.toLowerCase() &&
    (connection.params.username || '') === target.username &&
    (connection.params.port || defaultPort(connection.protocol)) ===
      (target.port || defaultPort(target.protocol)),
  );
}

function quickCandidate(query, visible) {
  const target = quickAddress(query);
  if (!target || (!target.explicit && visible.length) || exactConnection(target)) return null;
  return target;
}

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

function renderRecent() {
  const recent = sorted(connections.filter(connection =>
    connection.id !== active.id && recentUsed.has(connection.id),
  )).slice(0, 2);
  const nav = $('recent-connections');
  nav.hidden = recent.length === 0;
  nav.replaceChildren(...recent.map(connection => el('button', {
    class: 'btn btn--quiet btn--small recent-connections__button',
    type: 'button',
    'aria-label': '最近連線 ' + connection.name,
    onclick: () => handlers.onOpen(connection),
  }, connection.name)));
}

function card(connection) {
  // The open button's visible text is just the name (its accessible name "連線 <name>" contains it).
  // The badge and endpoint sit beside it; CSS stretches the button over the whole card.
  const isCurrent = connection.id === active.id;
  const status = statuses.get(connection.id) || { dot: 'idle', text: '未連線' };
  const statusBadge = el(
          'span',
          { class: 'card__status' },
          el('span', { class: 'dot', 'data-state': status.dot, 'aria-hidden': 'true' }),
          el('span', {}, status.text),
        );
  const trusted = connection.params['host-key'] || connection.params['server-cert-sha256'];
  return el(
    'article',
    { class: 'card', 'data-active': isCurrent ? 'true' : null },
    el(
      'div',
      { class: 'card__title' },
      el('span', { class: 'badge', 'data-protocol': connection.protocol }, protocolLabel(connection.protocol)),
      el(
        'button',
        {
          class: 'card__open',
          type: 'button',
          'aria-label': '連線 ' + connection.name,
          onkeydown: event => {
            if ((event.ctrlKey || event.altKey) && event.key.toLowerCase() === 'd') {
              event.preventDefault();
              handlers.onDuplicate(connection);
            }
          },
          onclick: () => {
            handlers.onOpen(connection);
          },
        },
        connection.name,
      ),
      statusBadge,
    ),
    el('p', { class: 'card__meta' }, endpoint(connection), trusted ? ' · 已信任' : ''),
    el(
      'div',
      { class: 'card__actions' },
      el('button', {
        class: 'btn btn--quiet btn--small', type: 'button',
        'aria-label': (pinned.has(connection.id) ? '取消釘選 ' : '釘選 ') + connection.name,
        'aria-pressed': String(pinned.has(connection.id)),
        onclick: () => {
          if (pinned.has(connection.id)) pinned.delete(connection.id);
          else pinned.add(connection.id);
          saveSet('cracamole-pinned', pinned);
          renderLibrary();
          const label = (pinned.has(connection.id) ? '取消釘選 ' : '釘選 ') + connection.name;
          [...$('connections-list').querySelectorAll('button')]
            .find(button => button.getAttribute('aria-label') === label)?.focus();
        },
      }, pinned.has(connection.id) ? '★' : '☆'),
      el('button', {
        class: 'btn btn--quiet btn--small', type: 'button',
        'aria-label': '複製 ' + connection.name,
        onclick: () => handlers.onDuplicate(connection),
      }, '⧉'),
      el(
        'button',
        { class: 'btn btn--quiet btn--small', type: 'button', 'aria-label': '編輯 ' + connection.name, onclick: () => handlers.onEdit(connection) },
        '✎',
      ),
      el(
        'button',
        { class: 'btn btn--quiet btn--small', type: 'button', 'aria-label': '刪除 ' + connection.name, onclick: () => handlers.onDelete(connection) },
        '🗑',
      ),
    ),
  );
}

export function renderLibrary() {
  const searchText = $('library-search').value.trim();
  const query = searchText.toLowerCase();
  const visible = sorted(connections).filter(connection => matches(connection, query));
  const groups = new Map();
  for (const connection of visible) {
    const group = pinned.has(connection.id) ? '★ 常用釘選' : groupName(connection);
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group).push(connection);
  }
  const names = [...groups.keys()].sort((a, b) => {
    if (a === '★ 常用釘選') return -1;
    if (b === '★ 常用釘選') return 1;
    return a.localeCompare(b, 'zh-Hant');
  });
  const elements = names.map(name => {
    const members = groups.get(name);
    const group = el('details', { class: 'conn-group', open: Boolean(query) || !collapsed.has(name) },
      el('summary', { class: 'conn-group__title' }, name + '（' + members.length + '）'),
      el('div', { class: 'conn-group__items' }, ...members.map(card)));
    group.addEventListener('toggle', () => {
      if ($('library-search').value.trim()) return;
      if (group.open) collapsed.delete(name);
      else collapsed.add(name);
      saveSet('cracamole-collapsed-groups', collapsed);
    });
    return group;
  });
  $('connections-list').replaceChildren(...elements);
  renderRecent();
  const quick = quickCandidate(searchText, visible);
  $('quick-connect').hidden = !quick;
  $('quick-connect').textContent = quick
    ? '快速設定 ' + (quick.username ? quick.username + '@' : '') + quick.hostname +
      (quick.port ? ':' + quick.port : '')
    : '';
  $('no-match').hidden = visible.length > 0 || connections.length === 0;
  return names.flatMap(name => groups.get(name));
}

export function setConnections(list) {
  connections = list;
  renderLibrary();
}

export function setActive(id, dot = 'idle', text = '') {
  if (active.id && active.id !== id && ['connected', 'connecting'].includes(active.dot)) {
    statuses.set(active.id, { dot: 'closed', text: '連線已結束' });
  }
  if (id && dot === 'connecting') rememberRecent(id);
  if (id && text) statuses.set(id, { dot, text });
  active = { id, dot, text };
  renderLibrary();
}

export function initLibrary(callbacks) {
  handlers = callbacks;
  pinned = readSet('cracamole-pinned');
  collapsed = readSet('cracamole-collapsed-groups');
  for (const id of [...readSet('cracamole-recent')].slice(-50)) recentUsed.set(id, ++sequence);
  $('add-connection').addEventListener('click', () => handlers.onAdd());
  const search = $('library-search');
  search.addEventListener('input', renderLibrary);
  search.addEventListener('keydown', event => {
    if (event.key === 'ArrowDown') {
      const first = $('connections-list').querySelector('details[open] .card__open');
      if (first) {
        event.preventDefault();
        first.focus();
      }
      return;
    }
    if (event.key === 'Enter') {
      const visible = renderLibrary();
      const quick = quickCandidate(search.value.trim(), visible);
      const target = quickAddress(search.value.trim());
      const exact = target?.explicit ? exactConnection(target) : null;
      if (quick) {
        event.preventDefault();
        handlers.onQuickConnect(quick);
      } else if (exact) {
        event.preventDefault();
        handlers.onOpen(exact);
      } else if (visible.length > 0) {
        event.preventDefault();
        handlers.onOpen(visible[0]);
      }
    }
  });
  $('quick-connect').addEventListener('click', () => {
    const target = quickCandidate(search.value.trim(), renderLibrary());
    if (target) handlers.onQuickConnect(target);
  });
  $('connections-list').addEventListener('keydown', event => {
    if (!['ArrowDown', 'ArrowUp'].includes(event.key)) return;
    const buttons = [...$('connections-list').querySelectorAll('details[open] .card__open')];
    const index = buttons.indexOf(document.activeElement);
    if (index < 0) return;
    const next = buttons[index + (event.key === 'ArrowDown' ? 1 : -1)];
    if (next) {
      event.preventDefault();
      next.focus();
    }
  });
  document.addEventListener('keydown', event => {
    const shortcut = event.key.toLowerCase() === 'k' && (event.ctrlKey || event.metaKey) && !event.altKey;
    const slash = event.key === '/' && !event.ctrlKey && !event.metaKey && !event.altKey;
    if (!shortcut && !slash) return;
    if ($('workspace').hidden || document.querySelector('dialog[open]')) return;
    const tag = document.activeElement?.tagName;
    if (slash && (['INPUT', 'TEXTAREA', 'SELECT'].includes(tag) || document.activeElement === $('terminal'))) return;
    event.preventDefault();
    setDrawerOpen(true);
    search.focus();
  });
}
