// Explicit server identity approval. Discovery never authenticates to the remote host.
import { api, ApiError } from '../api.js';
import { $ } from '../util/dom.js';
import { formatFingerprint } from '../util/format.js';
import { toast } from '../ui/toast.js';

export function identityText(identity) {
  const lines = identity.kind === 'ssh'
    ? ['SSH 主機金鑰', identity.public_key, 'SHA-256 指紋：', identity.fingerprint]
    : ['主機：' + identity.subject, '簽發者：' + identity.issuer,
      'SHA-256 指紋：', formatFingerprint(identity.sha256)];
  return lines.join(String.fromCharCode(10));
}

export async function inspectIdentity(target, isCurrent, showOverlay) {
  const ssh = target.connection.protocol === 'ssh';
  const path = ssh ? '/host-key' : '/certificate';
  let identity;
  try {
    identity = await api('/connections/' + target.connection.id + path, { method: 'POST' });
  } catch (error) {
    if (isCurrent()) {
      showOverlay({
        title: '無法確認主機身分',
        detail: error instanceof ApiError ? error.message : '探測失敗，請確認主機與連接埠後再試。',
        tone: 'error', canEdit: true, canRetry: true,
      });
    }
    return;
  }
  if (!isCurrent()) return;
  identity.kind = ssh ? 'ssh' : 'rdp';
  const trusted = ssh ? identity.trusted : Boolean(identity.pinned);
  const matches = ssh ? identity.matches : identity.matches_pin;
  if (trusted && matches) return;
  target.identity = identity;
  showOverlay({
    title: trusted ? (ssh ? '主機金鑰已變更或遭撤銷' : '憑證已變更')
      : (ssh ? '尚未信任 SSH 主機金鑰' : '無法驗證伺服器憑證'),
    detail: trusted
      ? '遠端身分與已信任的設定不符，或金鑰已遭撤銷。可能是重新安裝、更換金鑰，或中途攔截。請向管理員核對下方指紋；確認原因後，到「編輯連線」更新信任設定。'
      : '請把下方 SHA-256 指紋與伺服器管理員透過其他可信管道提供的指紋比對。確認相同後才能信任；儲存後，之後的連線只接受這個身分。',
    tone: 'error', canTrust: !trusted, canEdit: true, identity,
  });
}

export async function saveIdentity(target, isCurrent, onTrusted) {
  if (!target?.identity) return;
  const button = $('overlay-trust');
  button.disabled = true;
  try {
    const { connection, identity } = target;
    const latest = await api('/connections/' + connection.id);
    if (!isCurrent()) return;
    if (latest.protocol !== connection.protocol
      || latest.params.hostname !== connection.params.hostname
      || latest.params.port !== connection.params.port) {
      toast('主機設定已變更，請重新連線並核對新的指紋。');
      return;
    }
    const parameter = identity.kind === 'ssh'
      ? { 'host-key': identity.public_key }
      : { 'server-cert-sha256': identity.sha256 };
    const saved = await api('/connections/' + connection.id, {
      method: 'PUT',
      body: { name: latest.name, protocol: latest.protocol, params: { ...latest.params, ...parameter } },
    });
    if (isCurrent()) await onTrusted(saved);
  } catch (error) {
    toast(error instanceof ApiError ? error.message : '無法儲存主機信任設定，請再試一次。');
  } finally {
    button.disabled = false;
  }
}
