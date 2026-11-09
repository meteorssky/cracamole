import { api, ApiError } from '../api.js';
import { $ } from '../util/dom.js';
import { openDialog } from '../ui/dialog.js';
import { defaultPort, protocolArgs, quickAddress } from '../util/protocols.js';
import {
  collapseAllOptions,
  fillOptions,
  readOptions,
  renderOptions,
  updateOptionSummaries,
  validateOptions,
} from './options.js';

const SECRET_FIELDS = ['password', 'private-key', 'passphrase'];
const FIELD_ERROR_IDS = {
  name: 'error-name',
  protocol: 'error-protocol',
  port: 'error-port',
  hostname: 'error-hostname',
  'server-cert-sha256': 'error-server-cert-sha256',
};

// Parameters owned by the protocol-specific form groups (or fixed values). A value is written
// only when the chosen protocol declares that parameter.
const FORM_PARAMS = [
  ['domain', f => f.elements.domain.value.trim()],
  ['security', f => f.elements.security.value],
  ['ignore-cert', f => (f.elements['ignore-cert'].checked ? 'true' : 'false')],
  ['server-cert-sha256', f => f.elements['server-cert-sha256'].value.trim()],
  ['host-key', f => f.elements['host-key'].value.trim()],
  ['login-success-regex', f => f.elements['login-success-regex'].value],
  ['login-failure-regex', f => f.elements['login-failure-regex'].value],
  ['terminal-type', () => 'xterm'],
  ['func-keys-and-keypad', () => 'vt100'],
];

let editing = null;
let cloneSource = null;
let connectAfterSave = false;
let onSaved = async () => {};

const form = () => $('connection-form');

function applyProtocol() {
  const f = form();
  const protocol = f.elements.protocol.value;
  for (const group of f.querySelectorAll('[data-protocols]')) {
    group.hidden = !group.dataset.protocols.split(' ').includes(protocol);
  }
  $('f-port').placeholder = defaultPort(protocol);
  updateOptionSummaries(f, protocol);
}

function clearErrors() {
  for (const id of Object.values(FIELD_ERROR_IDS)) {
    const node = $(id);
    node.hidden = true;
    node.textContent = '';
  }
  for (const input of form().querySelectorAll('[aria-invalid]')) input.removeAttribute('aria-invalid');
  $('editor-error').hidden = true;
}

function showError(field, message) {
  const id = FIELD_ERROR_IDS[field] || (field && document.getElementById('error-' + field) ? 'error-' + field : null);
  if (!id) {
    $('editor-error').textContent = message;
    $('editor-error').hidden = false;
    return;
  }
  const node = $(id);
  node.textContent = message;
  node.hidden = false;
  const input = field === 'protocol'
    ? form().querySelector('input[name="protocol"]:checked') || form().querySelector('input[name="protocol"]')
    : form().elements[field];
  if (!input) return;
  input.setAttribute('aria-invalid', 'true');
  input.closest('details')?.setAttribute('open', ''); // never leave an error inside a collapsed section
  input.focus();
}

// Same rules as the server, so people get feedback without a round trip.
function validate(f) {
  const name = f.elements.name.value.trim();
  if (!name || name.length > 120) return ['name', '請輸入 1 到 120 個字元的連線名稱。'];
  if (!f.elements.hostname.value.trim()) return ['hostname', '請填寫主機名稱或 IP（最長 255 字元）。'];
  const port = f.elements.port.value.trim();
  if (port && !(/^[0-9]+$/.test(port) && Number(port) >= 1 && Number(port) <= 65535)) {
    return ['port', '連接埠須為 1 到 65535 的數字，或留空使用預設值。'];
  }
  return validateOptions(f, f.elements.protocol.value);
}

function buildParams(f, protocol) {
  const params = { ...(editing?.params || cloneSource?.params || {}) };
  for (const key of ['hostname', 'port', 'username']) params[key] = f.elements[key].value.trim();
  for (const key of SECRET_FIELDS) {
    const value = f.elements[key].value;
    if (value || f.elements['clear-secrets'].checked) params[key] = value;
  }
  const accepted = protocolArgs(protocol);
  for (const [name, read] of FORM_PARAMS) {
    if (accepted.has(name)) params[name] = read(f);
  }
  if (cloneSource && (
    protocol !== cloneSource.protocol ||
    params.hostname.toLowerCase() !== (cloneSource.params.hostname || '').trim().toLowerCase() ||
    (params.port || defaultPort(protocol)) !==
      ((cloneSource.params.port || '').trim() || defaultPort(cloneSource.protocol))
  )) {
    delete params['host-key'];
    delete params['server-cert-sha256'];
  }
  // The server rejects parameters a protocol does not declare, so never carry over leftovers
  // from another protocol the connection used before.
  for (const key of Object.keys(params)) {
    if (!accepted.has(key)) delete params[key];
  }
  return params;
}

function fillSecretHints(connection) {
  const configured = connection?.configured_secrets || [];
  const hints = { password: 'hint-password', 'private-key': 'hint-private-key' };
  for (const secret of SECRET_FIELDS) {
    form().elements[secret].placeholder = configured.includes(secret) ? '已設定；留空保留' : '';
  }
  const passwordHint = $(hints.password);
  passwordHint.textContent = configured.includes('password') ? '已設定密碼。留空會保留目前的密碼；若變更主機、連接埠或憑證驗證設定，須重新輸入。' : '';
  passwordHint.hidden = !configured.includes('password');
  const keyHint = $(hints['private-key']);
  keyHint.textContent = configured.includes('private-key') ? '已設定私鑰。留空會保留目前的私鑰；若變更主機或連接埠，須重新輸入。' : '';
  keyHint.hidden = !configured.includes('private-key');
  $('clear-secrets-row').hidden = configured.length === 0;
}

export function openEditor(connection = null, protocol = null) {
  cloneSource = null;
  editing = connection;
  const f = form();
  f.reset();
  clearErrors();
  $('editor-title').textContent = connection ? '編輯連線' : '新增連線';
  $('duplicate-connection').hidden = !connection;
  if (connection) {
    f.elements.name.value = connection.name;
    f.elements.protocol.value = connection.protocol;
  } else if (protocol) {
    f.elements.protocol.value = protocol;
  }
  const params = connection?.params || {};
  for (const name of ['hostname', 'port', 'username', 'host-key', 'login-success-regex', 'login-failure-regex', 'domain', 'server-cert-sha256']) {
    f.elements[name].value = params[name] || '';
  }
  fillOptions(f, params, Boolean(connection));
  f.elements.security.value = params.security || 'any';
  f.elements['ignore-cert'].checked = params['ignore-cert'] === 'true';
  fillSecretHints(connection);
  applyProtocol();
  openDialog($('connection-editor'), f.elements.name);
}

export function openQuickEditor(target) {
  openEditor(null, target.protocol);
  const f = form();
  f.elements.hostname.value = target.hostname;
  f.elements.port.value = target.port;
  f.elements.username.value = target.username;
  f.elements.name.value = (target.username ? target.username + '@' : '')
    .concat(target.hostname).slice(0, 120);
  f.elements.password.focus();
}

function duplicateName(name) {
  const match = name.match(/^(.*?)(\d+)$/u);
  if (!match) return (name + ' - 複製').slice(0, 120);
  const next = (BigInt(match[2]) + 1n).toString().padStart(match[2].length, '0');
  return (match[1] + next).slice(0, 120);
}

export function duplicateConnection(connection) {
  const publicParams = Object.fromEntries(
    Object.entries(connection.params || {}).filter(([key]) => !SECRET_FIELDS.includes(key)),
  );
  const source = { ...connection, params: publicParams, configured_secrets: [] };
  openEditor(source);
  editing = null;
  cloneSource = source;
  $('editor-title').textContent = '複製連線';
  $('duplicate-connection').hidden = true;
  form().elements.name.value = duplicateName(connection.name);
  fillSecretHints(null);
  form().elements.hostname.focus();
  form().elements.hostname.select();
}

export function initEditor({ saved }) {
  onSaved = saved;
  const optionsBox = $('protocol-options');
  renderOptions(optionsBox);
  const dialog = $('connection-editor');
  const syncSummaries = () => updateOptionSummaries(form(), form().elements.protocol.value);
  optionsBox.addEventListener('input', syncSummaries);
  optionsBox.addEventListener('change', syncSummaries);
  optionsBox.addEventListener('toggle', syncSummaries, true);
  $('collapse-options').addEventListener('click', () => {
    collapseAllOptions(form(), form().elements.protocol.value);
  });
  $('close-editor').addEventListener('click', () => dialog.close());
  $('cancel-editor').addEventListener('click', () => dialog.close());
  $('duplicate-connection').addEventListener('click', () => {
    if (!editing) return;
    const source = editing;
    dialog.addEventListener('close', () => duplicateConnection(source), { once: true });
    dialog.close();
  });
  $('save-and-connect').addEventListener('click', () => {
    connectAfterSave = true;
    form().requestSubmit();
  });
  dialog.addEventListener('close', () => {
    editing = null;
    cloneSource = null;
    connectAfterSave = false;
    form().reset();
  });
  form().addEventListener('change', event => {
    if (event.target.name === 'protocol') applyProtocol();
  });
  form().elements.hostname.addEventListener('paste', event => {
    const text = event.clipboardData?.getData('text/plain') || '';
    if (/^(?:[a-z][a-z0-9+.-]*:\/\/)?[^@/\s]*:[^@/\s]*@/i.test(text.trim())) {
      event.preventDefault();
      showError('hostname', '連線位址不可包含密碼，請在密碼欄位輸入。');
      return;
    }
    const address = quickAddress(text);
    if (!address?.explicit) return;
    event.preventDefault();
    const f = form();
    if (address.protocolExplicit) {
      f.elements.protocol.value = address.protocol;
      applyProtocol();
    }
    f.elements.hostname.value = address.hostname;
    if (address.username) f.elements.username.value = address.username;
    if (address.port) f.elements.port.value = address.port;
    clearErrors();
  });
  form().addEventListener('submit', async event => {
    event.preventDefault();
    const f = event.target;
    const shouldConnect = connectAfterSave;
    connectAfterSave = false;
    clearErrors();
    if (!editing && !f.elements.name.value.trim() && f.elements.hostname.value.trim()) {
      const user = f.elements.username.value.trim();
      f.elements.name.value = Array.from((user ? user + '@' : '') + f.elements.hostname.value.trim())
        .slice(0, 120).join('');
    }
    const problem = validate(f);
    if (problem) {
      showError(...problem);
      return;
    }
    const protocol = f.elements.protocol.value;
    const button = $('save-connection');
    const connectBtn = $('save-and-connect');
    button.disabled = true;
    connectBtn.disabled = true;
    button.setAttribute('aria-busy', 'true');
    try {
      const params = buildParams(f, protocol);
      readOptions(f, protocol, params);
      const body = { name: f.elements.name.value.trim(), protocol, params };
      const savedConn = await api('/connections' + (editing ? '/' + editing.id : ''), {
        method: editing ? 'PUT' : 'POST',
        body,
      });
      dialog.close();
      await onSaved(savedConn, { connect: shouldConnect });
    } catch (failure) {
      if (failure instanceof ApiError) showError(failure.field, failure.message);
      else showError(null, '儲存失敗，請再試一次。');
    } finally {
      button.disabled = false;
      connectBtn.disabled = false;
      button.removeAttribute('aria-busy');
    }
  });
}
