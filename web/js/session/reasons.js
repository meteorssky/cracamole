// Turns "how a connection ended" into a message a person can act on:
// what happened, the likely cause, and what to do next.

const GENERIC_MESSAGE = 'Aborted. See logs.';

// Guacamole status codes (decimal) used by the server.
const AUTH = new Set([769, 771]); // unauthorized, forbidden
const TIMEOUT = new Set([514, 522, 523, 776]);
const NOT_FOUND = new Set([516, 519]);
const UPSTREAM = new Set([515, 520, 521]);
const BUSY = new Set([513, 797]);

/**
 * @param {{ wasConnected: boolean, userInitiated: boolean, failure: ?{code:number,message:string} }} input
 * @returns {{ reason: string, status: string, title: string, detail: string, tone: 'neutral'|'error', canRetry: boolean, canEdit: boolean }}
 */
export function classify({ wasConnected, userInitiated, failure }) {
  if (userInitiated) {
    return { reason: 'user', status: '未連線', title: '', detail: '', tone: 'neutral', canRetry: false, canEdit: false };
  }
  // The server's generic text carries no information; only show a message that adds something.
  const informative = failure?.message && failure.message !== GENERIC_MESSAGE;
  const technical = informative ? '技術細節：' + failure.message : '';
  const code = failure?.code;
  const make = (reason, title, detail, extra = {}) => ({
    reason,
    status: '連線失敗',
    title,
    detail: [detail, technical].filter(Boolean).join(' '),
    tone: 'error',
    canRetry: true,
    canEdit: false,
    ...extra,
  });

  if (AUTH.has(code)) {
    return make('auth', '驗證失敗', '遠端拒絕了這組帳號或密碼。請確認帳號與密碼，必要時編輯連線後再試。', { canEdit: true });
  }
  if (code === 783) {
    return make('encoding', '文字格式或編碼不支援',
      '要傳送的文字無法用選定編碼表示，或剪貼簿格式不支援。請編輯連線，確認遠端使用的文字編碼；新系統建議使用 UTF-8。',
      { canEdit: true, canRetry: false });
  }
  if (TIMEOUT.has(code)) {
    return make('timeout', '遠端沒有回應', '連線逾時。主機可能已關機、位址或連接埠有誤，或被防火牆擋住。請確認後再試一次。', { canEdit: true });
  }
  if (NOT_FOUND.has(code)) {
    return make('not-found', '找不到主機', '無法解析或找不到這個主機名稱。請確認拼字與 DNS，或改用 IP 位址。', { canEdit: true });
  }
  if (UPSTREAM.has(code)) {
    return make('upstream', '無法連線到遠端', '遠端主機拒絕連線或無法使用。請確認服務已啟動、連接埠正確。', { canEdit: true });
  }
  if (BUSY.has(code)) {
    return make('busy', '伺服器忙碌', '同時連線數已達上限。請先關閉不用的連線，稍後再試。');
  }
  if (failure) {
    return make('server', '連線發生錯誤', '伺服器回報了無法處理的錯誤。請稍後再試，若持續發生請查看伺服器日誌。');
  }
  if (!wasConnected) {
    return make('network', '無法建立連線', '連不到 cracamole 伺服器，或連線在建立前就中斷了。請檢查網路後重新連線。');
  }
  return {
    reason: 'remote-exit',
    status: '連線已結束',
    title: '連線已結束',
    detail: '你可能執行了 exit 或登出，或遠端伺服器關閉了連線。需要時可以重新連線。',
    tone: 'neutral',
    canRetry: true,
    canEdit: false,
  };
}
