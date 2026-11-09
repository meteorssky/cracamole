// System logs viewer view: displays recent logs from the backend daemon.
import { api, ApiError } from '../api.js';
import { toast } from '../ui/toast.js';
import { $, el } from '../util/dom.js';
import { openDialog } from '../ui/dialog.js';

let timer = null;
let rawLogs = [];
let autoScroll = true;

function formatTime(isoString) {
  if (!isoString) return '';
  try {
    const d = new Date(isoString);
    if (isNaN(d.getTime())) return isoString;
    return d.toLocaleTimeString('zh-TW', {
      hour12: false,
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    }) + '.' + String(d.getMilliseconds()).padStart(3, '0');
  } catch {
    return isoString;
  }
}

function matchesFilter(entry, query, minLevel) {
  if (minLevel) {
    const severities = { ERROR: 5, WARNING: 4, WARN: 4, INFO: 3, DEBUG: 2, TRACE: 1 };
    const entrySev = severities[entry.level?.toUpperCase()] || 0;
    const targetSev = severities[minLevel.toUpperCase()] || 0;
    if (entrySev < targetSev) return false;
  }
  if (query) {
    const q = query.toLowerCase();
    const hitMsg = entry.message?.toLowerCase().includes(q);
    const hitTarget = entry.target?.toLowerCase().includes(q);
    const hitLevel = entry.level?.toLowerCase().includes(q);
    if (!hitMsg && !hitTarget && !hitLevel) return false;
  }
  return true;
}

function renderLogs() {
  const query = $('logs-search')?.value.trim() || '';
  const minLevel = $('logs-level-filter')?.value || '';
  const list = $('logs-list');
  const empty = $('logs-empty');
  const count = $('logs-count');
  if (!list) return;

  const filtered = rawLogs.filter(entry => matchesFilter(entry, query, minLevel));

  list.replaceChildren();
  if (filtered.length === 0) {
    empty.hidden = false;
  } else {
    empty.hidden = true;
    for (const entry of filtered) {
      const item = el(
        'div',
        { class: 'log-entry', 'data-level': entry.level },
        el('span', { class: 'log-entry__time' }, formatTime(entry.timestamp)),
        el('span', { class: 'log-entry__badge' }, entry.level),
        el('span', { class: 'log-entry__target' }, entry.target || ''),
        el('span', { class: 'log-entry__message' }, entry.message || '')
      );
      list.appendChild(item);
    }
  }

  if (count) {
    count.textContent = query || minLevel ? `${filtered.length} / ${rawLogs.length} 條` : `${rawLogs.length} 條`;
  }

  if (autoScroll) {
    const container = $('logs-container');
    if (container) {
      container.scrollTop = container.scrollHeight;
    }
  }
}

async function fetchLogs() {
  const status = $('logs-status');
  try {
    const res = await api('/logs?limit=500');
    if (res?.logs) {
      rawLogs = res.logs;
      renderLogs();
      if (status) {
        const now = new Date().toLocaleTimeString('zh-TW', { hour12: false });
        status.textContent = `最後更新：${now}`;
      }
    }
  } catch (error) {
    if (status) {
      status.textContent = '取得日誌失敗：' + (error instanceof ApiError ? error.message : '連線異常');
    }
  }
}

function startPolling() {
  stopPolling();
  const auto = $('logs-auto-refresh')?.checked;
  if (auto) {
    timer = setInterval(fetchLogs, 3000);
  }
}

function stopPolling() {
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
}

export function openLogs() {
  const dialog = $('logs-dialog');
  if (!dialog) return;
  autoScroll = true;
  openDialog(dialog, $('close-logs'));
  fetchLogs();
  startPolling();
}

export function closeLogs() {
  const dialog = $('logs-dialog');
  if (!dialog) return;
  stopPolling();
  dialog.close();
}

export function initLogs() {
  const dialog = $('logs-dialog');
  if (!dialog) return;

  $('close-logs')?.addEventListener('click', closeLogs);
  $('close-logs-btn')?.addEventListener('click', closeLogs);
  $('refresh-logs')?.addEventListener('click', () => {
    fetchLogs();
    toast('已重新整理日誌');
  });

  $('logs-auto-refresh')?.addEventListener('change', () => {
    if ($('logs-auto-refresh').checked) startPolling();
    else stopPolling();
  });

  $('logs-search')?.addEventListener('input', () => {
    autoScroll = false;
    renderLogs();
  });

  $('logs-level-filter')?.addEventListener('change', () => {
    autoScroll = false;
    renderLogs();
  });

  const container = $('logs-container');
  if (container) {
    container.addEventListener('scroll', () => {
      // 若滾動條距離底部小於 20px，則維持自動滾動
      const atBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 20;
      autoScroll = atBottom;
    });
  }

  $('copy-logs')?.addEventListener('click', async () => {
    const query = $('logs-search')?.value.trim() || '';
    const minLevel = $('logs-level-filter')?.value || '';
    const filtered = rawLogs.filter(entry => matchesFilter(entry, query, minLevel));
    if (filtered.length === 0) {
      toast('沒有可複製的日誌');
      return;
    }
    const text = filtered.map(e => e.raw || `[${e.timestamp}] [${e.level}] ${e.target}: ${e.message}`).join('\n');
    try {
      await navigator.clipboard.writeText(text);
      toast(`已複製 ${filtered.length} 條日誌到剪貼簿`);
    } catch {
      toast('瀏覽器不允許自動寫入剪貼簿');
    }
  });

  dialog.addEventListener('close', stopPolling);
}
