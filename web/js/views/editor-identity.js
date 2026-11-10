// Probe the current endpoint without transmitting credentials or saving trust automatically.
import { api, ApiError } from '../api.js';
import { $ } from '../util/dom.js';
import { formatFingerprint } from '../util/format.js';
import { defaultPort, identityKind } from '../util/protocols.js';
import { confirmAction } from '../ui/dialog.js';

let getEditing = () => null;
let generation = 0;
let busy = false;

function snapshot(connection) {
  const f = $('connection-form');
  const protocol = f.elements.protocol.value;
  const key = protocol === 'ssh' ? 'host-key' : 'server-cert-sha256';
  return {
    protocol,
    hostname: f.elements.hostname.value.trim(),
    port: f.elements.port.value.trim() || defaultPort(protocol),
    pin: f.elements[key].value.trim(),
    id: connection?.id,
  };
}

function sameSaved(view, connection) {
  if (!connection || connection.id !== view.id || connection.protocol !== view.protocol) return false;
  const key = view.protocol === 'ssh' ? 'host-key' : 'server-cert-sha256';
  return view.hostname === (connection.params.hostname || '').trim()
    && view.port === ((connection.params.port || '').trim() || defaultPort(view.protocol))
    && view.pin === (connection.params[key] || '').trim();
}

export function refreshEditorIdentity() {
  const connection = getEditing();
  const view = snapshot(connection);
  const supported = identityKind(view.protocol);
  $('editor-identity-section').hidden = !supported;
  $('editor-identity-probe').disabled = busy || !supported || !validEndpoint(view);
}

function validEndpoint(view) {
  return view.hostname.length > 0 && new TextEncoder().encode(view.hostname).length <= 255
    && !view.hostname.includes('\0') && /^[0-9]+$/.test(view.port)
    && Number(view.port) >= 1 && Number(view.port) <= 65535;
}

export function initEditorIdentity(current) {
  getEditing = current;
  const form = $('connection-form');
  const dialog = $('connection-editor');
  const result = $('editor-identity-result');
  const apply = $('editor-identity-use');
  let proposal = null;
  const invalidate = () => {
    generation++;
    result.hidden = true;
    result.textContent = '';
    apply.hidden = true;
    proposal = null;
    refreshEditorIdentity();
  };
  form.addEventListener('input', invalidate);
  form.addEventListener('change', invalidate);
  dialog.addEventListener('close', invalidate);
  apply.addEventListener('click', async () => {
    const candidate = proposal;
    if (!candidate || candidate.token !== generation) return;
    const confirmed = await confirmAction({ title: '確認主機身分',
      message: '請確認已透過其他可信管道核對指紋。填入後仍需儲存連線才會保存信任設定。',
      confirmLabel: '已核對，填入設定' });
    if (!confirmed || proposal !== candidate || candidate.token !== generation || !dialog.open) return;
    const now = snapshot(getEditing());
    if (now.id !== candidate.view.id || now.protocol !== candidate.view.protocol
      || now.hostname !== candidate.view.hostname || now.port !== candidate.view.port || now.pin !== '') return;
    if (candidate.saved) {
      try {
        const latest = await api('/connections/' + encodeURIComponent(now.id));
        if (proposal !== candidate || candidate.token !== generation || !dialog.open || !sameSaved(now, latest)) return;
      } catch {
        if (candidate.token === generation) result.textContent = '無法確認已保存的設定，請重新探測。';
        return;
      }
    }
    const field = form.elements[now.protocol === 'ssh' ? 'host-key' : 'server-cert-sha256'];
    field.value = candidate.value;
    field.dispatchEvent(new Event('input', { bubbles: true }));
    result.hidden = false;
    result.textContent = '已填入信任欄位；儲存連線後才會生效。';
  });
  $('editor-identity-probe').addEventListener('click', async () => {
    const connection = getEditing();
    const view = snapshot(connection);
    if (!identityKind(view.protocol) || !validEndpoint(view) || busy) return;
    const token = ++generation;
    proposal = null;
    apply.hidden = true;
    busy = true;
    refreshEditorIdentity();
    result.hidden = false;
    result.textContent = '正在探測主機身分…';
    const currentView = () => {
      const now = snapshot(getEditing());
      return dialog.open && token === generation && now.id === view.id
        && now.protocol === view.protocol && now.hostname === view.hostname
        && now.port === view.port && now.pin === view.pin;
    };
    try {
      const ssh = view.protocol === 'ssh';
      if (!sameSaved(view, connection)) {
        const identity = await api('/identity-probe', {
          method: 'POST', body: { protocol: view.protocol, hostname: view.hostname, port: view.port },
        });
        if (!currentView()) return;
        if (identity.hostname !== view.hostname || String(identity.port) !== String(Number(view.port))) {
          result.textContent = '探測目的地已變更，請重新操作。';
          return;
        }
        const fingerprint = ssh ? identity.fingerprint : formatFingerprint(identity.sha256);
        const rejected = ssh && identity.trusted && !identity.matches;
        result.textContent = rejected
          ? `SHA-256 指紋：${fingerprint}。與既有主機金鑰不符或已撤銷，無法填入信任。`
          : `SHA-256 指紋：${fingerprint}。尚未比對已保存信任；請透過其他可信管道核對。`;
        const value = ssh ? identity.public_key : identity.sha256;
        const key = ssh ? 'host-key' : 'server-cert-sha256';
        const originalPin = connection?.protocol === view.protocol
          && connection.params.hostname?.trim() === view.hostname
          && (connection.params.port?.trim() || defaultPort(view.protocol)) === view.port
          && connection.params[key];
        if (!rejected && !view.pin && !originalPin && typeof value === 'string' && value) {
          proposal = { token, view, value, saved: false };
          apply.hidden = false;
        }
        return;
      }
      const latest = await api('/connections/' + encodeURIComponent(view.id));
      if (!currentView()) return;
      if (!sameSaved(view, latest)) {
        result.textContent = '已保存的設定已變更，請重新開啟編輯器。';
        return;
      }
      const identity = await api('/connections/' + encodeURIComponent(view.id)
        + (ssh ? '/host-key' : '/certificate'), { method: 'POST' });
      const after = await api('/connections/' + encodeURIComponent(view.id));
      if (!currentView()) return;
      if (!sameSaved(view, after)) {
        result.textContent = '已保存的設定已變更，請重新開啟編輯器。';
        return;
      }
      if (identity.hostname !== view.hostname || String(identity.port) !== String(Number(view.port))) {
        result.textContent = '已保存的目的地已變更，請重新開啟編輯器。';
        return;
      }
      const trusted = ssh ? identity.trusted : Boolean(identity.pinned);
      const matches = ssh ? identity.matches : identity.matches_pin;
      const state = trusted ? (matches ? '與已保存的信任相符' : '與已保存的信任不符，請勿直接信任') : '尚未保存信任';
      const fingerprint = ssh ? identity.fingerprint : formatFingerprint(identity.sha256);
      result.textContent = `SHA-256 指紋：${fingerprint}。${state}；請透過其他可信管道核對。`;
      const value = ssh ? identity.public_key : identity.sha256;
      if (!trusted && !view.pin && typeof value === 'string' && value) {
        proposal = { token, view, value, saved: true };
        apply.hidden = false;
      }
    } catch (error) {
      if (currentView()) result.textContent = error instanceof ApiError ? error.message : '無法探測主機身分。';
    } finally {
      busy = false;
      refreshEditorIdentity();
    }
  });
}
