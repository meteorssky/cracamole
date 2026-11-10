// The stage: the remote display plus everything about its state (overlay, status bar, helper keys, clipboard).
import { $, el } from '../util/dom.js';
import { identityText, inspectIdentity, saveIdentity } from './trust.js';
import { endpoint, identityKind, protocolLabel } from '../util/protocols.js';
import { createSession, loadGuacamole } from '../session/guac.js';
import { CTRL_ALT_DEL, KEYS } from '../session/keys.js';
import { toast } from '../ui/toast.js';
import { confirmAction } from '../ui/dialog.js';
import { setChromeCollapsed, setDrawerOpen, setDrawerOverlay } from './drawer.js';

let current = null; // { connection, session, attempt, identity }
let attempts = 0;
let confirmationPending = false;
let hooks = { onEdit: () => {}, onActive: () => {}, onTrusted: () => {} };

function setStatus(text, dotState) {
  $('terminal-status').textContent = text;
  for (const id of ['status-dot', 'chip-dot', 'pill-dot', 'overlay-dot']) $(id).dataset.state = dotState;
  $('overlay-state').textContent = text;
  $('pill-status').textContent = text;
  hooks.onActive(current?.connection.id ?? null, dotState, text);
}

function setControls(connected) {
  for (const id of ['toggle-clipboard', 'toggle-keybar', 'collapse-chrome']) $(id).disabled = !connected;
  const params = current?.connection.params || {};
  const canPaste = connected && params['read-only'] !== 'true' && params['disable-paste'] !== 'true';
  $('quick-paste').disabled = !canPaste;
  $('paste-clipboard').disabled = !canPaste;
  $('disconnect').disabled = !current;
  if (!connected) {
    $('clipboard-panel').hidden = true;
    $('toggle-clipboard').setAttribute('aria-expanded', 'false');
  }
}

function hideClipboard() {
  $('clipboard-panel').hidden = true;
  $('toggle-clipboard').setAttribute('aria-expanded', 'false');
}

function pasteFromPanel() {
  if (current?.session?.paste($('clipboard-text').value)) {
    hideClipboard();
    current.session.focus();
  }
}

function showOverlay({
  connecting = false, title, detail = '', tone = 'neutral',
  canRetry = false, canEdit = false, canTrust = false, identity = null,
}) {
  $('overlay-spinner').hidden = !connecting;
  const heading = $('overlay-title');
  heading.textContent = title;
  heading.dataset.tone = tone;
  $('overlay-detail').textContent = detail;
  $('overlay-detail').hidden = !detail;
  const cert = $('overlay-cert');
  cert.hidden = !identity;
  cert.textContent = identity ? identityText(identity) : '';
  $('overlay-trust').textContent =
    identity?.kind === 'host-key' ? '信任這把主機金鑰並重新連線' : '信任這張憑證並重新連線';
  $('overlay-trust').hidden = !canTrust;
  $('overlay-retry').hidden = !canRetry;
  $('overlay-edit').hidden = !canEdit;
  $('overlay-close').hidden = false;
  $('overlay').hidden = false;
  $('overlay').setAttribute('role', tone === 'error' ? 'alert' : 'status');
}

function hideOverlay() {
  $('overlay').hidden = true;
}

function renderKeybar(modifiers = { ctrl: false, alt: false }) {
  const buttons = KEYS.map(key =>
    el(
      'button',
      {
        class: 'btn btn--small',
        type: 'button',
        'aria-label': key.aria,
        'aria-pressed': key.modifier ? String(Boolean(modifiers[key.modifier])) : null,
        onmousedown: event => event.preventDefault(),
        onclick: () => {
          if (!current?.session) return;
          if (key.modifier) current.session.toggleModifier(key.modifier);
          else current.session.sendKey(key.keysym);
        },
      },
      key.label,
    ),
  );
  $('keybar').replaceChildren(...buttons);
}

function setKeybarVisible(visible) {
  $('keybar').hidden = !visible;
  $('toggle-keybar').setAttribute('aria-pressed', String(visible));
}

function onSessionState(connection, state, info) {
  if (!current || current.connection.id !== connection.id) return;
  current.state = state;
  if (state === 'connecting') {
    setStatus('正在連線…', 'connecting');
    showOverlay({ connecting: true, title: '正在連線到 ' + connection.name + '…' });
    return;
  }
  if (state === 'connected') {
    hideOverlay();
    setStatus('已連線', 'connected');
    setControls(true);
    current.session.focus();
    return;
  }
  setControls(false);
  setKeybarVisible(false);
  setChromeCollapsed(false);
  if (info.reason === 'user') {
    setStatus('未連線', 'idle');
    return;
  }
  setStatus(info.status, info.tone === 'error' ? 'error' : 'closed');
  showOverlay(explainMissingCredentials(connection, info));
  if (
    info.reason === 'upstream' &&
    (identityKind(connection.protocol) === 'host-key' ||
      (identityKind(connection.protocol) === 'certificate' &&
        (connection.params['ignore-cert'] !== 'true' || connection.params['server-cert-sha256'])))
  ) {
    const target = current;
    inspectIdentity(target, () => current === target, showOverlay,
      status => setStatus(status, 'error'));
  }
}

// Windows enables NLA by default, which needs the account before the desktop appears.
function explainMissingCredentials(connection, info) {
  if (info.reason !== 'auth' || connection.protocol !== 'rdp') return info;
  const noUser = !connection.params.username;
  const noPassword = !(connection.configured_secrets || []).includes('password');
  if (!noUser && !noPassword) {
    const slash = String.fromCharCode(92);
    return {
      ...info,
      detail:
        info.detail + ' Windows 帳號若屬於網域，請在「編輯連線」填寫網域，或把帳號寫成「網域' + slash + '帳號」。',
    };
  }
  return {
    ...info,
    title: '這條連線還沒有設定' + (noUser ? '帳號' : '密碼'),
    detail:
      'Windows 預設要求先通過網路層級驗證（NLA），必須在連線前提供帳號與密碼。請到「編輯連線」填寫' +
      (noUser && noPassword ? '帳號與密碼' : noUser ? '帳號' : '密碼') +
      '後再連線。',
    canEdit: true,
  };
}

export async function openSession(connection) {
  if (current?.state === 'connected') {
    if (current.connection.id === connection.id) {
      setDrawerOpen(false);
      current.session.focus();
      return;
    }
    const target = current;
    if (confirmationPending) return;
    confirmationPending = true;
    let confirmed;
    try {
      confirmed = await confirmAction({
        title: '切換連線？',
        message: '切換至「' + connection.name + '」會中斷「' + target.connection.name + '」。',
        confirmLabel: '中斷並切換',
      });
    } finally {
      confirmationPending = false;
    }
    if (!confirmed || current !== target) return;
  }
  closeSession({ keepHistory: true });
  current = { connection, session: null, attempt: ++attempts, identity: null, state: 'connecting' };
  if (history.state?.cracamole !== 'session') {
    history.replaceState({ cracamole: 'menu' }, '');
    history.pushState({ cracamole: 'session', id: connection.id }, '', '#session');
  }
  setDrawerOverlay(true);
  setDrawerOpen(false);
  $('terminal-placeholder').hidden = true;
  $('session-chip').hidden = false;
  $('back-to-menu').hidden = false;
  $('disconnect').disabled = false;
  const meta = protocolLabel(connection.protocol) + ' · ' + endpoint(connection);
  $('terminal-name').textContent = connection.name;
  $('pill-name').textContent = connection.name;
  $('status-meta').textContent = meta;
  $('overlay-endpoint').textContent = meta;
  $('send-cad').hidden = connection.protocol !== 'rdp';
  renderKeybar();
  const target = current;
  setControls(false);
  onSessionState(connection, 'connecting');
  try {
    await loadGuacamole();
  } catch {
    if (current !== target) return;
    current.state = 'closed';
    setStatus('載入失敗', 'error');
    showOverlay({
      title: '無法載入連線元件',
      detail: '請檢查網路或重新整理頁面，然後再試一次。',
      tone: 'error',
      canRetry: true,
    });
    return;
  }
  if (current !== target) return;
  current.session = createSession({
    connection,
    mount: $('terminal'),
    onState: (state, info) => onSessionState(connection, state, info),
    onClipboard: text => { $('clipboard-text').value = text; },
    onClipboardError: message => toast(message),
    onModifiers: modifiers => renderKeybar(modifiers),
    onEscape: () => {
      setChromeCollapsed(false);
      $('toggle-library').focus();
    },
  });
  if (window.matchMedia('(pointer: coarse)').matches) setKeybarVisible(true);
}

export function closeSession({ fromHistory = false, keepHistory = false } = {}) {
  if (!current) return;
  current.session?.dispose();
  current = null;
  if (!fromHistory && !keepHistory && location.hash === '#session') {
    history.replaceState({ cracamole: 'menu' }, '', location.pathname + location.search);
  }
  hideOverlay();
  setControls(false);
  setKeybarVisible(false);
  setChromeCollapsed(false);
  setDrawerOverlay(false);
  $('terminal-placeholder').hidden = false;
  $('session-chip').hidden = true;
  $('back-to-menu').hidden = true;
  $('send-cad').hidden = true;
  $('status-meta').textContent = '';
  $('overlay-endpoint').textContent = '';
  $('clipboard-text').value = '';
  setStatus('未連線', 'idle');
  setDrawerOpen(true);
}

export function activeConnectionId() {
  return current?.connection.id ?? null;
}

export function showEmptyGuidance(hasConnections) {
  $('empty-actions').hidden = hasConnections;
  $('empty-title').textContent = hasConnections ? '選擇一台主機' : '新增你的第一台主機';
  $('empty-text').textContent = hasConnections
    ? '從左側連線庫選擇主機，終端機會在這裡開啟。'
    : '選擇協定並填寫主機資訊，儲存後就能從這裡連線。';
}

export function initStage(callbacks) {
  hooks = callbacks;
  setStatus('未連線', 'idle');
  if (!history.state?.cracamole) history.replaceState({ cracamole: 'menu' }, '');
  window.addEventListener('popstate', async () => {
    if (!current) {
      setDrawerOpen(true);
      return;
    }
    if (current.state !== 'connected') {
      closeSession({ fromHistory: true });
      return;
    }
    const target = current;
    history.pushState({ cracamole: 'session', id: target.connection.id }, '', '#session');
    if (confirmationPending) return;
    confirmationPending = true;
    let confirmed;
    try {
      confirmed = await confirmAction({
        title: '返回連線庫？',
        message: '返回連線庫會中斷「' + target.connection.name + '」。',
        confirmLabel: '中斷並返回',
      });
    } finally {
      confirmationPending = false;
    }
    if (confirmed && current === target) closeSession();
  });
  $('overlay-trust').addEventListener('click', () => {
    const target = current;
    saveIdentity(target, () => current === target, hooks.onTrusted);
  });
  $('overlay-retry').addEventListener('click', () => current && openSession(current.connection));
  $('overlay-edit').addEventListener('click', () => current && hooks.onEdit(current.connection));
  $('overlay-close').addEventListener('click', () => closeSession());
  $('disconnect').addEventListener('click', () => closeSession());
  $('back-to-menu').addEventListener('click', () => closeSession());
  $('pill-back').addEventListener('click', () => closeSession());
  $('toggle-keybar').addEventListener('click', () => setKeybarVisible($('keybar').hidden));
  $('send-cad').addEventListener('click', () => current?.session?.sendCombo(CTRL_ALT_DEL));
  $('toggle-clipboard').addEventListener('click', () => {
    const panel = $('clipboard-panel');
    panel.hidden = !panel.hidden;
    $('toggle-clipboard').setAttribute('aria-expanded', String(!panel.hidden));
    if (!panel.hidden) $('clipboard-text').focus();
  });
  $('paste-clipboard').addEventListener('click', pasteFromPanel);
  $('clipboard-panel').addEventListener('keydown', event => {
    if (event.key === 'Escape') {
      event.preventDefault();
      hideClipboard();
      current?.session?.focus();
    } else if (event.key === 'Enter' && event.ctrlKey && !$('paste-clipboard').disabled) {
      event.preventDefault();
      pasteFromPanel();
    }
  });
  $('quick-paste').addEventListener('click', async () => {
    const target = current;
    if (!target || target.state !== 'connected') return;
    try {
      const text = await navigator.clipboard.readText();
      if (current === target && target.state === 'connected' && target.session.paste(text)) {
        hideClipboard();
        target.session.focus();
      }
    } catch {
      toast('無法讀取本機剪貼簿，請開啟剪貼簿面板後手動貼入。');
    }
  });
  $('copy-clipboard').addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText($('clipboard-text').value);
      toast('已複製');
    } catch {
      $('clipboard-text').select();
      toast('瀏覽器不允許自動複製，請手動複製所選內容。');
    }
  });
  window.addEventListener('beforeunload', event => {
    if (current?.state !== 'connected') return;
    event.preventDefault();
    event.returnValue = '';
  });
  window.addEventListener('pagehide', () => current?.session?.dispose());
}
