import { api, ApiError } from '../api.js';
import { $ } from '../util/dom.js';
import { openDialog } from '../ui/dialog.js';
import { DEFAULT_PORTS } from '../util/protocols.js';
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

// Parameters that only make sense for one protocol; removed when another protocol is saved.
const SSH_ONLY = ['host-key', 'private-key', 'passphrase'];
const TELNET_ONLY = ['login-success-regex', 'login-failure-regex'];
const RDP_ONLY = ['domain', 'security', 'ignore-cert', 'server-cert-sha256'];
const TERMINAL_ONLY = ['terminal-type', 'func-keys-and-keypad'];

let editing = null;
let connectAfterSave = false;
let onSaved = async () => {};

const form = () => $('connection-form');

function applyProtocol() {
  const f = form();
  const protocol = f.elements.protocol.value;
  for (const group of f.querySelectorAll('[data-protocols]')) {
    group.hidden = !group.dataset.protocols.split(' ').includes(protocol);
  }
  $('f-port').placeholder = DEFAULT_PORTS[protocol] || '';
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
  const input = form().elements[field];
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
  const params = { ...(editing?.params || {}) };
  for (const key of ['hostname', 'port', 'username']) params[key] = f.elements[key].value.trim();
  for (const key of SECRET_FIELDS) {
    const value = f.elements[key].value;
    if (value || f.elements['clear-secrets'].checked) params[key] = value;
  }
  const drop = keys => {
    for (const key of keys) delete params[key];
  };
  if (protocol === 'rdp') {
    drop([...TERMINAL_ONLY, ...SSH_ONLY, ...TELNET_ONLY]);
    params.domain = f.elements.domain.value.trim();
    params.security = f.elements.security.value;
    params['ignore-cert'] = f.elements['ignore-cert'].checked ? 'true' : 'false';
    params['server-cert-sha256'] = f.elements['server-cert-sha256'].value.trim();
    return params;
  }
  drop(RDP_ONLY);
  params['terminal-type'] = 'xterm';
  params['func-keys-and-keypad'] = 'vt100';
  if (protocol === 'ssh') {
    drop(TELNET_ONLY);
    params['host-key'] = f.elements['host-key'].value.trim();
  } else {
    drop(SSH_ONLY);
    params['login-success-regex'] = f.elements['login-success-regex'].value;
    params['login-failure-regex'] = f.elements['login-failure-regex'].value;
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
  editing = connection;
  const f = form();
  f.reset();
  clearErrors();
  $('editor-title').textContent = connection ? '編輯連線' : '新增連線';
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
  $('save-and-connect').addEventListener('click', () => {
    connectAfterSave = true;
    form().requestSubmit();
  });
  dialog.addEventListener('close', () => {
    editing = null;
    connectAfterSave = false;
    form().reset();
  });
  form().elements.protocol.addEventListener('change', applyProtocol);
  form().addEventListener('submit', async event => {
    event.preventDefault();
    const f = event.target;
    const shouldConnect = connectAfterSave;
    connectAfterSave = false;
    clearErrors();
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
