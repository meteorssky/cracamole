import { $, el } from '../util/dom.js';
import { defaultPort, endpoint, protocolLabel, quickAddress } from '../util/protocols.js';
import { setDrawerOpen } from './drawer.js';

let connections = [];
let active = { id: null, dot: 'idle', text: '' };
const recentUsed = new Map();
const statuses = new Map();
let pinned = new Set();
let collapsed = new Set();
let selectedFilter = 'all';
let sequence = 0;
let handlers = {};
const PINNED_GROUP = JSON.stringify(['pinned']);
const UNGROUPED = JSON.stringify(['ungrouped']);
const namedGroup = name => JSON.stringify(['group', name]);
const COLLAPSED_KEY = 'cracamole-collapsed-groups-v2';

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
  return slash > 0 ? connection.name.slice(0, slash).trim() || null : null;
}

function displayName(connection, inPinnedGroup) {
  if (inPinnedGroup || groupName(connection) === null) return connection.name;
  return connection.name.slice(connection.name.indexOf('/') + 1).trim() || connection.name;
}

function readCollapsed() {
  try {
    if (localStorage.getItem(COLLAPSED_KEY) !== null) return readSet(COLLAPSED_KEY);
  } catch {
    return new Set();
  }
  const migrated = new Set();
  for (const name of readSet('cracamole-collapsed-groups')) {
    migrated.add(namedGroup(name));
    if (name === '未分類') migrated.add(UNGROUPED);
    if (name === '★ 常用釘選') migrated.add(PINNED_GROUP);
  }
  saveSet(COLLAPSED_KEY, migrated);
  return migrated;
}

function exactConnections(target) {
  return connections.filter(connection =>
    connection.protocol === target.protocol &&
    (connection.params.hostname || '').toLowerCase() === target.hostname.toLowerCase() &&
    (connection.params.username || '') === target.username &&
    (target.protocol !== 'rdp' || (connection.params.domain || '') === target.domain) &&
    (connection.params.port || defaultPort(connection.protocol)) ===
      (target.port || defaultPort(target.protocol)),
  );
}

function quickCandidate(query, visible) {
  const target = quickAddress(query);
  if (!target || (!target.explicit && visible.length) || exactConnections(target).length) return null;
  return target;
}

function matches(connection, query) {
  if (!query) return true;
  const haystack = [
    connection.name,
    groupName(connection) || '未分類',
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

function matchesFilter(connection) {
  return selectedFilter === 'all' ||
    (selectedFilter === 'pinned' && pinned.has(connection.id)) ||
    (selectedFilter === 'recent' && recentUsed.has(connection.id)) ||
    (selectedFilter === 'connected' && statuses.get(connection.id)?.dot === 'connected') ||
    connection.protocol === selectedFilter;
}

function groupKeys() {
  return new Set(connections.filter(matchesFilter).map(connection => {
    if (pinned.has(connection.id)) return PINNED_GROUP;
    const name = groupName(connection);
    return name === null ? UNGROUPED : namedGroup(name);
  }));
}

function updateGroupToggle() {
  const groups = groupKeys();
  const toggle = $('toggle-groups');
  toggle.disabled = !groups.size || Boolean($('library-search').value.trim());
  toggle.textContent = groups.size && [...groups].every(key => collapsed.has(key))
    ? '全部展開' : '全部折疊';
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

function card(connection, inPinnedGroup) {
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
    { class: 'card', 'data-connection-id': connection.id, 'data-pinned-group': inPinnedGroup && groupName(connection) ? 'true' : null, 'data-active': isCurrent ? 'true' : null },
    el(
      'div',
      { class: 'card__title' },
      el('span', { class: 'badge', 'data-protocol': connection.protocol }, protocolLabel(connection.protocol)),
      el(
        'button',
        {
          class: 'card__open',
          'data-action': 'open',
          type: 'button',
          'aria-label': '連線 ' + connection.name,
          onclick: () => {
            handlers.onOpen(connection);
          },
        },
        displayName(connection, inPinnedGroup),
      ),
      statusBadge,
    ),
    el('p', { class: 'card__meta' },
      el('span', { class: 'card__endpoint' }, endpoint(connection)),
      trusted ? el('span', { class: 'card__policy' },
        status.text === '身分不符' ? '已保存指紋' : '已信任') : null,
      connection.params['read-only'] === 'true'
        ? el('span', { class: 'card__policy' }, '唯讀') : null),
    el(
      'div',
      { class: 'card__actions' },
      el('button', {
        class: 'btn btn--quiet btn--small', type: 'button',
        'data-action': 'pin',
        'aria-label': (pinned.has(connection.id) ? '取消釘選 ' : '釘選 ') + connection.name,
        'aria-pressed': String(pinned.has(connection.id)),
        onclick: () => {
          if (pinned.has(connection.id)) pinned.delete(connection.id);
          else pinned.add(connection.id);
          saveSet('cracamole-pinned', pinned);
          renderLibrary();
          const updated = [...$('connections-list').querySelectorAll('[data-connection-id]')]
            .find(item => item.dataset.connectionId === connection.id);
          const button = updated?.querySelector('button[data-action="pin"]');
          if (button?.closest('details')?.open) button.focus({ preventScroll: true });
          else $('library-search').focus({ preventScroll: true });
        },
      }, pinned.has(connection.id) ? '★' : '☆'),
      inPinnedGroup && groupName(connection) ? el('button', {
        class: 'btn btn--quiet btn--small', type: 'button', 'data-action': 'rename-group',
        'aria-label': '重新命名群組 ' + groupName(connection),
        onclick: () => handlers.onRenameGroup(groupName(connection)),
      }, '組') : null,
      el('button', {
        class: 'btn btn--quiet btn--small', type: 'button',
        'data-action': 'duplicate',
        'aria-label': '複製 ' + connection.name,
        onclick: () => handlers.onDuplicate(connection),
      }, '⧉'),
      el('button', {
        class: 'btn btn--quiet btn--small', type: 'button',
        'data-action': 'export',
        'aria-label': '匯出 ' + connection.name + ' 的設定',
        onclick: () => handlers.onExport(connection),
      }, '⇧'),
      el(
        'button',
        { class: 'btn btn--quiet btn--small', type: 'button', 'data-action': 'edit', 'aria-label': '編輯 ' + connection.name, onclick: () => handlers.onEdit(connection) },
        '✎',
      ),
      el(
        'button',
        { class: 'btn btn--quiet btn--small', type: 'button', 'data-action': 'delete', 'aria-label': '刪除 ' + connection.name, onclick: () => handlers.onDelete(connection) },
        '🗑',
      ),
    ),
  );
}

export function renderLibrary() {
  const list = $('connections-list');
  const focused = document.activeElement;
  const focusedCard = !$('library').hidden && list.contains(focused)
    ? focused.closest('[data-connection-id]') : null;
  const focusTarget = focusedCard && focused.dataset.action
    ? { id: focusedCard.dataset.connectionId, action: focused.dataset.action } : null;
  const focusedGroup = !$('library').hidden && list.contains(focused) && focused.dataset.groupAction
    ? focused.closest('[data-group-key]') : null;
  const groupFocusTarget = focusedGroup
    ? { key: focusedGroup.dataset.groupKey, action: focused.dataset.groupAction } : null;
  const searchText = $('library-search').value.trim();
  const query = searchText.toLowerCase();
  const target = quickAddress(searchText);
  const exact = target?.explicit ? exactConnections(target) : [];
  const exactIds = new Set(exact.map(connection => connection.id));
  const visible = sorted(connections).filter(connection =>
    (matches(connection, query) || exactIds.has(connection.id)) &&
    matchesFilter(connection));
  const groups = new Map();
  for (const connection of visible) {
    const name = groupName(connection);
    const key = pinned.has(connection.id) ? PINNED_GROUP : name === null ? UNGROUPED : namedGroup(name);
    const label = key === PINNED_GROUP ? '★ 常用釘選' : key === UNGROUPED ? '未分類（無群組）'
      : name === '★ 常用釘選' ? '群組：★ 常用釘選' : name;
    if (!groups.has(key)) groups.set(key, { label, name, members: [] });
    groups.get(key).members.push(connection);
  }
  const keys = [...groups.keys()].sort((a, b) => {
    if (a === PINNED_GROUP) return -1;
    if (b === PINNED_GROUP) return 1;
    return groups.get(a).label.localeCompare(groups.get(b).label, 'zh-Hant');
  });
  const elements = keys.map(key => {
    const { name, label, members } = groups.get(key);
    const connected = members.filter(member => statuses.get(member.id)?.dot === 'connected').length;
    const canAdd = key !== PINNED_GROUP && key !== UNGROUPED && name.length <= 116;
    const group = el('details', { class: 'conn-group', open: Boolean(query) || !collapsed.has(key) },
      el('summary', { class: 'conn-group__title', 'data-group-action': 'toggle' }, label + '（' + members.length + '）',
        connected ? el('span', {}, ' · ' + connected + ' 已連線') : null),
      el('div', { class: 'conn-group__items' }, ...members.map(connection =>
        card(connection, key === PINNED_GROUP))));
    group.addEventListener('toggle', () => {
      if (!group.isConnected || $('library-search').value.trim()) return;
      if (group.open) collapsed.delete(key);
      else collapsed.add(key);
      saveSet(COLLAPSED_KEY, collapsed);
      updateGroupToggle();
    });
    return el('div', { class: 'conn-group-container', 'data-group-key': key, 'data-named': canAdd ? 'true' : null }, group,
      canAdd ? el('div', { class: 'conn-group__tools' },
      el('button', { class: 'btn btn--quiet btn--small conn-group__add', type: 'button',
        'data-group-action': 'add',
        'aria-label': '新增至群組 ' + name, onclick: () => handlers.onAddGroup(name) }, '＋'),
      el('button', { class: 'btn btn--quiet btn--small conn-group__rename', type: 'button',
        'data-group-action': 'rename',
        'aria-label': '重新命名群組 ' + name, onclick: () => handlers.onRenameGroup(name) }, '✎')) : null);
  });
  list.replaceChildren(...elements);
  if (focusTarget) {
    const card = [...list.querySelectorAll('[data-connection-id]')]
      .find(item => item.dataset.connectionId === focusTarget.id);
    const button = card && [...card.querySelectorAll('button[data-action]')]
      .find(item => item.dataset.action === focusTarget.action);
    if (button && button.closest('details')?.open) button.focus({ preventScroll: true });
    else $('library-search').focus({ preventScroll: true });
  } else if (groupFocusTarget) {
    const group = [...list.querySelectorAll('[data-group-key]')]
      .find(item => item.dataset.groupKey === groupFocusTarget.key);
    const control = group && [...group.querySelectorAll('[data-group-action]')]
      .find(item => item.dataset.groupAction === groupFocusTarget.action);
    if (control) control.focus({ preventScroll: true });
    else $('library-search').focus({ preventScroll: true });
  }
  renderRecent();
  const quick = quickCandidate(searchText, visible);
  $('quick-connect').hidden = !quick;
  $('quick-connect').textContent = quick
    ? '快速設定 ' + (quick.username ? (quick.domain ? quick.domain + '\\' : '') + quick.username + '@' : '') + quick.hostname +
      (quick.port ? ':' + quick.port : '')
    : '';
  const hint = $('quick-hint');
  hint.textContent = exact.length > 1
    ? '有多筆符合的連線，請從清單選擇；必要時切換至「全部」。'
    : exact.length === 1 && !visible.some(connection => connection.id === exact[0].id)
      ? '符合的連線已被篩選，請切換至「全部」後選擇。' : '';
  hint.hidden = !hint.textContent;
  const noMatch = visible.length === 0 && connections.length > 0;
  const emptyMessage = query ? '沒有符合搜尋的連線。' :
    ({ pinned: '尚無釘選連線。', recent: '尚無最近連線。', connected: '目前沒有已連線的主機。' }[selectedFilter] || '沒有符合的連線。');
  $('no-match').textContent = emptyMessage;
  $('no-match').hidden = !noMatch;
  $('clear-library-filter').hidden = !noMatch || (!query && selectedFilter === 'all');
  updateGroupToggle();
  return keys.flatMap(key => groups.get(key).members);
}

export function setConnections(list) {
  connections = list;
  const ids = new Set(list.map(connection => connection.id));
  for (const id of statuses.keys()) {
    if (!ids.has(id)) statuses.delete(id);
  }
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
  collapsed = readCollapsed();
  for (const id of [...readSet('cracamole-recent')].slice(-50)) recentUsed.set(id, ++sequence);
  $('add-connection').addEventListener('click', () => handlers.onAdd());
  const search = $('library-search');
  $('library-filters').addEventListener('click', event => {
    const button = event.target.closest('button[data-filter]');
    if (!button) return;
    selectedFilter = button.dataset.filter;
    for (const item of $('library-filters').querySelectorAll('button[data-filter]')) {
      item.setAttribute('aria-pressed', String(item === button));
    }
    renderLibrary();
  });
  $('clear-library-filter').addEventListener('click', () => {
    selectedFilter = 'all';
    search.value = '';
    for (const item of $('library-filters').querySelectorAll('button[data-filter]')) {
      item.setAttribute('aria-pressed', String(item.dataset.filter === 'all'));
    }
    renderLibrary();
    search.focus();
  });
  $('toggle-groups').addEventListener('click', () => {
    const groups = groupKeys();
    const shouldCollapse = [...groups].some(key => !collapsed.has(key));
    for (const key of groups) {
      if (shouldCollapse) collapsed.add(key);
      else collapsed.delete(key);
    }
    saveSet(COLLAPSED_KEY, collapsed);
    renderLibrary();
  });
  search.addEventListener('input', renderLibrary);
  search.addEventListener('keydown', event => {
    if (['ArrowDown', 'ArrowUp'].includes(event.key) && !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) {
      const buttons = [...$('connections-list').querySelectorAll('details[open] .card__open')];
      const target = event.key === 'ArrowDown' ? buttons[0] : buttons.at(-1);
      if (target) {
        event.preventDefault();
        target.focus();
      }
      return;
    }
    if (event.key === 'Enter') {
      const visible = renderLibrary();
      const quick = quickCandidate(search.value.trim(), visible);
      const target = quickAddress(search.value.trim());
      const exact = target?.explicit ? exactConnections(target) : [];
      if (quick) {
        event.preventDefault();
        handlers.onQuickConnect(quick);
      } else if (exact.length === 1 && visible.some(connection => connection.id === exact[0].id)) {
        event.preventDefault();
        handlers.onOpen(exact[0]);
      } else if (exact.length === 0) {
        const first = $('connections-list').querySelector('details[open] .card__open');
        if (!first) return;
        event.preventDefault();
        first.click();
      }
    }
  });
  $('quick-connect').addEventListener('click', () => {
    const target = quickCandidate(search.value.trim(), renderLibrary());
    if (target) handlers.onQuickConnect(target);
  });
  $('connections-list').addEventListener('keydown', event => {
    const duplicateShortcut = event.key.toLowerCase() === 'd' &&
      (event.ctrlKey !== event.altKey) && !event.shiftKey && !event.metaKey;
    if (duplicateShortcut) {
      const card = event.target.closest('[data-connection-id]');
      const connection = card && connections.find(item => item.id === card.dataset.connectionId);
      if (connection) {
        event.preventDefault();
        handlers.onDuplicate(connection);
      }
      return;
    }
    if (!['ArrowDown', 'ArrowUp'].includes(event.key) || event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return;
    const buttons = [...$('connections-list').querySelectorAll('details[open] .card__open')];
    const current = event.target.closest('[data-connection-id]')?.querySelector('.card__open');
    const index = buttons.indexOf(current);
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
    if (shortcut) event.stopPropagation();
    setDrawerOpen(true);
    search.focus();
  }, true);
}

// 沿用已折疊群組的狀態；既有目標群組的狀態優先。
export function renameGroupState(oldName, newName) {
  const oldKey = namedGroup(oldName);
  if (!newName) {
    collapsed.delete(oldKey);
    saveSet(COLLAPSED_KEY, collapsed);
    return;
  }
  const newKey = namedGroup(newName);
  if (oldKey === newKey) return;
  if (collapsed.has(oldKey) && !connections.some(connection => groupName(connection) === newName)) collapsed.add(newKey);
  collapsed.delete(oldKey);
  saveSet(COLLAPSED_KEY, collapsed);
}
