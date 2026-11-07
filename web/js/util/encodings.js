import { el } from './dom.js';

export const TEXT_ENCODINGS = [
  ['', 'UTF-8（預設）'],
  ['Big5', 'Big5（繁體中文／CP950）'],
  ['GBK', 'GBK（簡體中文／CP936）'],
  ['GB18030', 'GB18030（簡體中文，含四位元組字元）'],
  ['Shift_JIS', 'Shift_JIS（日文／CP932）'],
  ['EUC-JP', 'EUC-JP（日文）'],
  ['EUC-KR', 'EUC-KR（韓文／CP949）'],
  ['windows-1251', 'Windows-1251（西里爾文字）'],
  ['windows-1252', 'Windows-1252（西歐文字）'],
];

// API callers may save accepted aliases such as CP950 or utf8. Keep their explicit choice
// when editing instead of silently resetting an unmatched <select> value to UTF-8.
export function preserveSavedEncoding(control, value) {
  for (const option of [...control.options]) {
    if (option.dataset.savedEncoding) option.remove();
  }
  if (value && ![...control.options].some(option => option.value === value)) {
    control.append(el('option', { value, 'data-saved-encoding': 'true' }, value + '（已儲存）'));
  }
}
