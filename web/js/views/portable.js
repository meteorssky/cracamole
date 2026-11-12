import { api, ApiError } from '../api.js';
import { state } from '../state.js';
import { $, el } from '../util/dom.js';
import { confirmAction, openDialog } from '../ui/dialog.js';
import { openImportedEditor } from './editor.js';

let resetPortable = () => {};
let startPortableExport = () => {};
export function clearPortable() { resetPortable(); }
export function openPortableExport(id) { startPortableExport(id); }

const message = error => error instanceof ApiError ? error.message : '操作失敗，請再試一次。';

export function initPortable({ onImported }) {
  const dialog = $('portable-dialog');
  const content = $('portable-content');
  const results = $('portable-results');
  const status = $('portable-status');
  const preview = $('portable-preview');
  const copy = $('portable-copy');
  const paste = $('portable-paste');
  const importAll = $('portable-import-all');
  const selection = $('portable-selection');
  let mode = '';
  let revision = 0;
  const invalidate = () => { revision++; preview.disabled = false; importAll.hidden = true; };
  const updateSelection = () => {
    const boxes = [...results.querySelectorAll('input[type="checkbox"]')];
    const count = boxes.filter(box => box.checked).length;
    $('portable-selection-count').textContent = `已選取 ${count} 筆（每次最多 100 筆）`;
    $('portable-select-first').disabled = boxes.length === 0;
    preview.disabled = count === 0 || count > 100;
  };
  const selectionChanged = () => {
    invalidate();
    content.value = '';
    copy.disabled = true;
    status.textContent = '';
    updateSelection();
  };
  resetPortable = () => {
    invalidate();
    if (dialog.open) dialog.close();
    mode = '';
    content.value = '';
    results.replaceChildren();
    status.textContent = '';
  };

  $('portable-close').addEventListener('click', () => { if (!importAll.disabled) dialog.close(); });
  dialog.addEventListener('cancel', event => { if (importAll.disabled) event.preventDefault(); });
  content.addEventListener('input', () => {
    if (mode === 'import') {
      invalidate();
      results.replaceChildren();
      status.textContent = '';
    }
  });
  $('import-connections').addEventListener('click', () => {
    if (mode !== 'import') {
      content.value = '';
      results.replaceChildren();
      status.textContent = '';
    }
    mode = 'import';
    selection.hidden = true;
    document.getElementById('portable-format-field').hidden = true;
    copy.hidden = true;
    paste.hidden = false;
    $('portable-title').textContent = '匯入連線設定';
    $('portable-help').textContent = '貼上交換資料並預覽。請核對主機、指紋及其他設定；每筆需另行檢查並保存，不會覆寫既有連線。';
    content.readOnly = false;
    preview.hidden = false;
    preview.disabled = false;
    openDialog(dialog, content);
  });
  startPortableExport = selectedId => {
    invalidate();
    mode = 'export';
    selection.hidden = false;
    importAll.hidden = true;
    document.getElementById('portable-format-field').hidden = false;
    copy.hidden = false;
    copy.disabled = true;
    paste.hidden = true;
    $('portable-title').textContent = '匯出連線設定';
    $('portable-help').textContent = '選擇 1 至 100 筆連線後產生交換資料。已保存的密碼、私鑰與密語不會匯出，但其他文字欄位可能含敏感內容，分享前請檢查。';
    content.value = '';
    content.readOnly = true;
    status.textContent = '';
    preview.hidden = false;
    preview.disabled = false;
    preview.textContent = '產生交換資料';
    const connections = state.get('connections') || [];
    results.replaceChildren(...connections.map(connection => el('label', {},
      el('input', { type: 'checkbox', value: connection.id, checked: selectedId ? connection.id === selectedId : connections.length <= 100 }),
      document.createTextNode(' ' + connection.name + ' (' + connection.protocol.toUpperCase() + ')'),
      el('br', {}),
    )));
    updateSelection();
    if (!connections.length) status.textContent = '目前沒有可匯出的連線。';
    openDialog(dialog, preview.disabled ? (connections.length ? $('portable-select-first') : $('portable-close')) : preview);
  };
  $('export-connections').addEventListener('click', () => startPortableExport());
  document.getElementById('portable-format').addEventListener('change', () => { selectionChanged(); });
  results.addEventListener('change', () => {
    if (mode === 'export') selectionChanged();
  });
  $('portable-select-first').addEventListener('click', () => {
    [...results.querySelectorAll('input[type="checkbox"]')].forEach((box, index) => { box.checked = index < 100; });
    selectionChanged();
  });
  $('portable-clear-selection').addEventListener('click', () => {
    for (const box of results.querySelectorAll('input[type="checkbox"]')) box.checked = false;
    selectionChanged();
  });
  copy.addEventListener('click', async () => {
    const requestRevision = revision;
    try {
      if (!navigator.clipboard?.writeText) throw new Error();
      await navigator.clipboard.writeText(content.value);
      if (requestRevision === revision && dialog.open) status.textContent = '已複製；請妥善保管交換資料。';
    } catch {
      if (requestRevision === revision && dialog.open) status.textContent = '無法寫入剪貼簿，請選取文字後手動複製。';
    }
  });
  paste.addEventListener('click', async () => {
    invalidate();
    results.replaceChildren();
    status.textContent = '';
    const requestRevision = revision;
    preview.disabled = true;
    try {
      if (!navigator.clipboard?.readText) throw new Error();
      const text = await navigator.clipboard.readText();
      if (requestRevision !== revision || !dialog.open) return;
      if (new TextEncoder().encode(text).length > 1024 * 1024) {
        status.textContent = '交換資料超過 1 MiB，請縮小內容。';
        return;
      }
      content.value = text;
      results.replaceChildren();
      status.textContent = '已貼上，請驗證並預覽。';
    } catch {
      if (requestRevision === revision && dialog.open) status.textContent = '無法讀取剪貼簿，請手動貼入文字。';
    } finally {
      if (requestRevision === revision) preview.disabled = false;
    }
  });
  importAll.addEventListener('click', async () => {
    const requestRevision = revision;
    const reviewedContent = content.value;
    const count = results.querySelectorAll('button').length;
    const confirmed = await confirmAction({
      title: '新增全部匯入設定？',
      message: '將新增 ' + count + ' 筆連線，保留資料內的公開信任指紋與安全選項。請先核對完整交換資料；不會覆寫現有連線。',
      confirmLabel: '新增全部',
    });
    if (!confirmed || requestRevision !== revision || !dialog.open || content.value !== reviewedContent) return;
    importAll.disabled = true;
    for (const button of results.querySelectorAll('button')) button.disabled = true;
    preview.disabled = true;
    content.disabled = true;
    paste.disabled = true;
    $('portable-close').disabled = true;
    try {
      const result = await api('/connections/import', { method: 'POST', body: { content: reviewedContent } });
      if (requestRevision !== revision || !dialog.open) return;
      results.replaceChildren();
      content.value = '';
      importAll.hidden = true;
      status.textContent = '已新增 ' + result.count + ' 筆連線。';
      await onImported();
    } catch (error) {
      if (requestRevision === revision && dialog.open) status.textContent = message(error);
    } finally {
      importAll.disabled = false;
      for (const button of results.querySelectorAll('button')) button.disabled = false;
      content.disabled = false;
      paste.disabled = false;
      $('portable-close').disabled = false;
      if (requestRevision === revision) preview.disabled = false;
    }
  });
  preview.addEventListener('click', async () => {
    const requestRevision = ++revision;
    preview.disabled = true;
    status.textContent = '';
    try {
      if (mode === 'export') {
        const ids = [...results.querySelectorAll('input:checked')].map(input => input.value);
        const response = await api('/connections/export', { method: 'POST', body: { ids, format: document.getElementById('portable-format').value } });
        if (requestRevision !== revision || !dialog.open) return;
        content.value = response.content;
        copy.disabled = false;
        status.textContent = '已產生交換資料；請檢查後複製。';
        content.focus();
        content.select();
      } else {
        const response = await api('/connections/import-preview', { method: 'POST', body: { content: content.value } });
        if (requestRevision !== revision || !dialog.open) return;
        results.replaceChildren(...response.connections.map(connection => el('div', {},
          el('button', { type: 'button', class: 'btn btn--small', onclick: () => {
            dialog.addEventListener('close', () => openImportedEditor(connection), { once: true });
            dialog.close();
          } }, '檢查並新增 ' + connection.name),
          document.createTextNode(' ' + connection.protocol.toUpperCase() + ' ' + (connection.params.hostname || '')),
        )));
        importAll.hidden = false;
        status.textContent = '已驗證 ' + response.connections.length + ' 筆；請逐筆檢查並保存。再次開啟匯入可繼續處理。';
      }
    } catch (error) {
      if (requestRevision === revision && dialog.open) status.textContent = message(error);
    } finally {
      if (requestRevision === revision) preview.disabled = false;
    }
  });
  dialog.addEventListener('close', () => { invalidate(); preview.textContent = '驗證並預覽'; });
}
