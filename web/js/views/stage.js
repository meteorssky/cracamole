// The stage: the remote display plus everything about its state (overlay, status bar, helper keys, clipboard).
import { api, ApiError } from '../api.js';
import { $, el } from '../util/dom.js';
import { formatFingerprint } from '../util/format.js';
import { endpoint, protocolLabel } from '../util/protocols.js';
import { createSession } from '../session/guac.js';
import { CTRL_ALT_DEL, KEYS } from '../session/keys.js';
import { toast } from '../ui/toast.js';
import { setDrawerOpen } from './drawer.js';

let current = null; // { connection, session, attempt, certificate }
let attempts = 0;
let hooks = { onEdit: () => {}, onActive: () => {}, onTrusted: () => {} };

function setStatus(text, dotState) {
  $('terminal-status').textContent = text;
  $('status-dot').dataset.state = dotState;
  $('chip-dot').dataset.state = dotState;
}

function setControls(connected) {
  for (const id of ['toggle-clipboard', 'toggle-keybar', 'disconnect']) $(id).disabled = !connected;
  if (!connected) {
    $('clipboard-panel').hidden = true;
    $('toggle-clipboard').setAttribute('aria-expanded', 'false');
  }
}

function certificateText(certificate) {
  return [
    '主機：' + certificate.subject,
    '簽發者：' + certificate.issuer,
    'SHA-256 指紋：',
    formatFingerprint(certificate.sha256),
  ].join(String.fromCharCode(10));
}

function showOverlay({
  connecting = false,
  title,
  detail = '',
  tone = 'neutral',
  canRetry = false,
  canEdit = false,
  canTrust = false,
  certificate = null,
}) {
  $('overlay-spinner').hidden = !connecting;
  const heading = $('overlay-title');
  heading.textContent = title;
  heading.dataset.tone = tone;
  $('overlay-detail').textContent = detail;
  $('overlay-detail').hidden = !detail;
  const cert = $('overlay-cert');
  cert.hidden = !certificate;
  cert.textContent = certificate ? certificateText(certificate) : '';
  $('overlay-trust').hidden = !canTrust;
  $('overlay-retry').hidden = !canRetry;
  $('overlay-edit').hidden = !canEdit;
  $('overlay-close').hidden = connecting;
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
        // keep the soft keyboard (and terminal focus) where it is while tapping helper keys
        onmousedown: event => event.preventDefault(),
        onclick: () => {
          if (!current) return;
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
  if (info.reason === 'user') {
    setStatus('未連線', 'idle');
    return;
  }
  setStatus(info.status, info.tone === 'error' ? 'error' : 'closed');
  showOverlay(explainMissingCredentials(connection, info));
  if (info.reason === 'upstream' && connection.protocol === 'rdp' && connection.params['ignore-cert'] !== 'true') {
    inspectCertificate(current);
  }
}

// Windows enables NLA by default, which needs the account before the desktop appears. A connection
// saved without a username fails authentication every time, so say that instead of "wrong password".
function explainMissingCredentials(connection, info) {
  if (info.reason !== 'auth' || connection.protocol !== 'rdp') return info;
  const noUser = !connection.params.username;
  const noPassword = !(connection.configured_secrets || []).includes('password');
  if (!noUser && !noPassword) {
    const slash = String.fromCharCode(92);
    return { ...info, detail: info.detail + ' Windows 帳號若屬於網域，請在「編輯連線」填寫網域，或把帳號寫成「網域' + slash + '帳號」。' };
  }
  return {
    ...info,
    title: '這條連線還沒有設定' + (noUser ? '帳號' : '密碼'),
    detail: 'Windows 預設要求先通過網路層級驗證（NLA），必須在連線前提供帳號與密碼。請到「編輯連線」填寫' + (noUser && noPassword ? '帳號與密碼' : noUser ? '帳號' : '密碼') + '後再連線。',
    canEdit: true,
  };
}

// An RDP connection that fails right after the TLS handshake is usually a certificate nobody vouched for.
// Ask the server what certificate the host presents so the person can decide, like an SSH host key.
async function inspectCertificate(target) {
  let certificate;
  try {
    certificate = await api('/connections/' + target.connection.id + '/certificate', { method: 'POST' });
  } catch {
    return; // the generic message stays
  }
  if (current !== target) return; // the person moved on
  if (certificate.pinned && !certificate.matches_pin) {
    target.certificate = certificate;
    showOverlay({
      title: '憑證已變更',
      detail: '這台主機現在出示的憑證，和你先前信任的不一樣。可能是伺服器重新安裝或更換憑證，也可能有人在中途攔截連線。請向伺服器管理員確認下方指紋；確定無誤後，到「編輯連線」更新已信任的指紋。',
      tone: 'error',
      canEdit: true,
      certificate,
    });
  } else if (!certificate.pinned) {
    target.certificate = certificate;
    showOverlay({
      title: '無法驗證伺服器憑證',
      detail: '這台 RDP 主機使用的是沒有受信任簽發者的憑證（Windows 預設就是這樣）。請把下方指紋和伺服器管理員提供的比對；確認相同後，就能信任這張憑證，之後只接受它。',
      tone: 'error',
      canTrust: true,
      canEdit: true,
      certificate,
    });
  }
}

async function trustCertificate() {
  const target = current;
  if (!target?.certificate) return;
  const { connection, certificate } = target;
  const button = $('overlay-trust');
  button.disabled = true;
  try {
    const saved = await api('/connections/' + connection.id, {
      method: 'PUT',
      body: {
        name: connection.name,
        protocol: connection.protocol,
        params: { ...connection.params, 'server-cert-sha256': certificate.sha256 },
      },
    });
    await hooks.onTrusted(saved);
  } catch (error) {
    toast(error instanceof ApiError ? error.message : '無法儲存信任的憑證，請再試一次。');
  } finally {
    button.disabled = false;
  }
}

export function openSession(connection) {
  closeSession();
  current = { connection, session: null, attempt: ++attempts, certificate: null };
  hooks.onActive(connection.id);
  setDrawerOpen(false);
  $('terminal-placeholder').hidden = true;
  $('session-chip').hidden = false;
  $('terminal-name').textContent = connection.name;
  $('status-meta').textContent = protocolLabel(connection.protocol) + ' · ' + endpoint(connection);
  $('send-cad').hidden = connection.protocol !== 'rdp';
  renderKeybar();
  const session = createSession({
    connection,
    mount: $('terminal'),
    onState: (state, info) => onSessionState(connection, state, info),
    onClipboard: text => {
      $('clipboard-text').value = text;
    },
    onModifiers: modifiers => renderKeybar(modifiers),
  });
  current.session = session;
  if (window.matchMedia('(pointer: coarse)').matches) setKeybarVisible(true);
}

export function closeSession() {
  if (!current) return;
  current.session?.dispose();
  current = null;
  hooks.onActive(null);
  hideOverlay();
  setControls(false);
  setKeybarVisible(false);
  $('terminal-placeholder').hidden = false;
  $('session-chip').hidden = true;
  $('send-cad').hidden = true;
  $('status-meta').textContent = '';
  $('clipboard-text').value = '';
  setStatus('未連線', 'idle');
  setDrawerOpen(true);
}

export function activeConnectionId() {
  return current?.connection.id ?? null;
}

// First-run guidance lives where the user is looking: in the empty stage.
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
  $('overlay-trust').addEventListener('click', trustCertificate);
  $('overlay-retry').addEventListener('click', () => current && openSession(current.connection));
  $('overlay-edit').addEventListener('click', () => current && hooks.onEdit(current.connection));
  $('overlay-close').addEventListener('click', closeSession);
  $('disconnect').addEventListener('click', closeSession);
  $('toggle-keybar').addEventListener('click', () => setKeybarVisible($('keybar').hidden));
  $('send-cad').addEventListener('click', () => current?.session.sendCombo(CTRL_ALT_DEL));
  $('toggle-clipboard').addEventListener('click', () => {
    const panel = $('clipboard-panel');
    panel.hidden = !panel.hidden;
    $('toggle-clipboard').setAttribute('aria-expanded', String(!panel.hidden));
    if (!panel.hidden) $('clipboard-text').focus();
  });
  $('paste-clipboard').addEventListener('click', () => {
    current?.session.paste($('clipboard-text').value);
    current?.session.focus();
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
  window.addEventListener('beforeunload', () => current?.session.dispose());
}
