// Helper keys for touch devices (a soft keyboard has no Esc/Tab/Ctrl/arrows).
// `keysym` values are X11 keysyms, which is what the Guacamole protocol carries.
export const KEYS = [
  { label: 'Esc', aria: 'Escape 鍵', keysym: 0xff1b },
  { label: 'Tab', aria: 'Tab 鍵', keysym: 0xff09 },
  { label: 'Ctrl', aria: 'Ctrl 修飾鍵（下一個按鍵生效）', modifier: 'ctrl' },
  { label: 'Alt', aria: 'Alt 修飾鍵（下一個按鍵生效）', modifier: 'alt' },
  { label: '↑', aria: '向上鍵', keysym: 0xff52 },
  { label: '↓', aria: '向下鍵', keysym: 0xff54 },
  { label: '←', aria: '向左鍵', keysym: 0xff51 },
  { label: '→', aria: '向右鍵', keysym: 0xff53 },
  { label: 'Home', aria: 'Home 鍵', keysym: 0xff50 },
  { label: 'End', aria: 'End 鍵', keysym: 0xff57 },
  { label: 'PgUp', aria: 'Page Up 鍵', keysym: 0xff55 },
  { label: 'PgDn', aria: 'Page Down 鍵', keysym: 0xff56 },
  { label: '|', aria: '直線符號', keysym: 0x7c },
  { label: '/', aria: '斜線符號', keysym: 0x2f },
  { label: '-', aria: '減號', keysym: 0x2d },
  { label: '~', aria: '波浪符號', keysym: 0x7e },
  { label: String.fromCharCode(92), aria: '反斜線符號', keysym: 0x5c },
  { label: '$', aria: '錢字符號', keysym: 0x24 },
];

export const MODIFIER_KEYSYMS = { ctrl: 0xffe3, alt: 0xffe9 };
export const CTRL_ALT_DEL = [0xffe3, 0xffe9, 0xffff];
