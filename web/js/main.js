import { api, ApiError, setUnauthorizedHandler } from './api.js';
import { state } from './state.js';
import { $, el } from './util/dom.js';
import { allProtocols, loadProtocols } from './util/protocols.js';
import { initTheme } from './util/theme.js';
import { confirmAction } from './ui/dialog.js';
import { toast } from './ui/toast.js';
import { initDrawer, setDrawerOpen } from './views/drawer.js';
import { duplicateConnection, openEditor, openGroupEditor, openQuickEditor, initEditor } from './views/editor.js';
import { initLibrary, setActive, setConnections } from './views/library.js';
import { hideLogin, initLogin, showLogin } from './views/login.js';
import { initLogs, openLogs } from './views/logs.js';
import { activeConnectionId, closeSession, initStage, openSession, showEmptyGuidance } from './views/stage.js';

async function refreshConnections() {
  const list = await api('/connections');
  state.set('connections', list);
  setConnections(list);
  showEmptyGuidance(list.length > 0);
}

// Protocol choices come from the server, so a new protocol needs no front-end change here.
function renderProtocolChoices() {
  const protocols = allProtocols();
  $('f-protocol').replaceChildren(...protocols.map((p, index) => el('label',
    { class: 'protocol-segment' },
    el('input', { class: 'protocol-segment__input', type: 'radio', name: 'protocol',
      value: p.name, checked: index === 0, 'aria-describedby': 'error-protocol' }),
    el('span', { class: 'protocol-segment__label' }, p.label),
  )));
  $('empty-actions').replaceChildren(
    ...protocols.map((p, index) =>
      el(
        'button',
        {
          class: index === 0 ? 'btn btn--primary' : 'btn',
          type: 'button',
          onClick: () => openEditor(null, p.name),
        },
        '新增 ' + p.label + ' 主機',
      ),
    ),
  );
}

async function enterWorkspace() {
  state.set('loggedIn', true);
  hideLogin();
  $('workspace').hidden = false;
  setDrawerOpen(true);
  try {
    await loadProtocols();
    renderProtocolChoices();
  } catch (error) {
    toast(error instanceof ApiError ? error.message : '無法載入協定清單，請重新整理頁面。');
  }
  await refreshConnections();
}

function leaveWorkspace(notice) {
  state.set('loggedIn', false);
  closeSession();
  setConnections([]);
  showLogin(notice);
}

async function deleteConnection(connection) {
  const confirmed = await confirmAction({
    title: '刪除「' + connection.name + '」？',
    message: '這會移除這台主機的連線設定與已保存的密碼，無法復原。',
    confirmLabel: '刪除這台主機',
  });
  if (!confirmed) return;
  try {
    await api('/connections/' + connection.id, { method: 'DELETE' });
    if (activeConnectionId() === connection.id) closeSession();
    await refreshConnections();
    toast('已刪除「' + connection.name + '」');
  } catch (error) {
    toast(error instanceof ApiError ? error.message : '刪除失敗，請再試一次。');
  }
}

function showInsecureWarning() {
  const local = ['localhost', '127.0.0.1', '[::1]'].includes(location.hostname);
  $('insecure-banner').hidden = location.protocol !== 'http:' || local;
}

async function boot() {
  initTheme();
  showInsecureWarning();
  setUnauthorizedHandler(() => {
    if (state.get('loggedIn')) leaveWorkspace('工作階段已到期或已登出，請重新登入。');
  });
  initDrawer();
  initLogin({ onLoggedIn: enterWorkspace });
  initEditor({
    saved: async (savedConn, options = {}) => {
      await refreshConnections();
      if (options.connect && savedConn) openSession(savedConn);
    },
  });
  initLibrary({
    onAdd: () => openEditor(),
    onAddGroup: group => openGroupEditor(group),
    onOpen: connection => openSession(connection),
    onEdit: connection => openEditor(connection),
    onDuplicate: duplicateConnection,
    onQuickConnect: openQuickEditor,
    onDelete: deleteConnection,
  });
  initStage({
    onEdit: connection => openEditor(connection),
    onActive: (id, dot, text) => setActive(id, dot, text),
    onTrusted: async connection => {
      await refreshConnections();
      openSession(connection);
    },
  });
  initLogs();
  $('open-logs')?.addEventListener('click', openLogs);
  $('logout').addEventListener('click', async () => {
    try {
      await api('/logout', { method: 'POST' });
      leaveWorkspace('');
    } catch (error) {
      toast(error instanceof ApiError ? error.message : '登出失敗，請再試一次。');
    }
  });
  try {
    await api('/session');
    await enterWorkspace();
  } catch {
    showLogin('');
  }
}

boot();
