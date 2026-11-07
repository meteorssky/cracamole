// Supported protocol settings: data only.
import { TEXT_ENCODINGS } from '../util/encodings.js';

const ALL = ['ssh', 'telnet', 'rdp', 'vnc'];

export const OPTION_SECTIONS = [
  {
    id: 'terminal',
    title: '終端機外觀與輸入',
    protocols: ['ssh', 'telnet'],
    fields: [
      { name: 'character-encoding', label: '文字編碼', type: 'select', options: TEXT_ENCODINGS,
        hint: '只影響遠端終端機文字，需配合主機設定；LANG 語系不等於編碼轉換。' },
      { name: 'font-size', label: '字型大小（pt）', type: 'number', min: 6, max: 72, placeholder: '12' },
      {
        name: 'color-scheme',
        label: '配色',
        type: 'select',
        options: [
          ['', '預設（灰字黑底）'],
          ['green-black', '綠字黑底'],
          ['white-black', '白字黑底'],
          ['black-white', '黑字白底'],
        ],
      },
      {
        name: 'scrollback',
        label: '捲動緩衝（行數）',
        type: 'number',
        min: 1,
        max: 100000,
        placeholder: '1000',
        hint: '行數越多，可往回捲的內容越多，也越占伺服器記憶體。',
      },
      {
        name: 'backspace',
        label: '退格鍵送出的代碼',
        type: 'select',
        options: [['', '預設（127，大多數系統）'], ['8', 'Ctrl+H（8，部分舊系統與設備）']],
      },
    ],
  },
  {
    id: 'ssh-session',
    title: 'SSH 連線選項',
    protocols: ['ssh'],
    fields: [
      {
        name: 'command',
        label: '連線後執行的指令',
        type: 'text',
        hint: '留空會開啟一般的登入 shell。填寫後會直接執行這個指令。',
      },
      {
        name: 'server-alive-interval',
        label: '保持連線間隔（秒）',
        type: 'number',
        min: 0,
        max: 3600,
        hint: '每隔幾秒送一次心跳，避免閒置時被防火牆或伺服器中斷。留空或 0 表示不送。',
      },
      { name: 'locale', label: '語系（LANG）', type: 'text', placeholder: '例如：zh_TW.UTF-8' },
    ],
  },
  {
    id: 'rdp-display',
    title: 'RDP 畫面與效能',
    protocols: ['rdp'],
    hint: '預設全部關閉，以節省頻寬並加快畫面。需要時再開啟。',
    fields: [
      { name: 'enable-wallpaper', label: '顯示桌布', type: 'checkbox' },
      { name: 'enable-font-smoothing', label: '字型平滑（ClearType）', type: 'checkbox' },
      { name: 'enable-full-window-drag', label: '拖曳視窗時顯示視窗內容', type: 'checkbox' },
      { name: 'enable-desktop-composition', label: '桌面特效（Aero）', type: 'checkbox' },
      { name: 'enable-menu-animations', label: '選單動畫', type: 'checkbox' },
    ],
  },
  {
    id: 'rdp-session',
    title: 'RDP 工作階段',
    protocols: ['rdp'],
    fields: [
      {
        name: 'client-name',
        label: '回報給遠端的電腦名稱',
        type: 'text',
        maxlength: 15,
        placeholder: 'cracamole',
        hint: '最多 15 個字元，會顯示在遠端的工作階段清單。',
      },
    ],
  },
  {
    id: 'vnc-display',
    title: 'VNC 畫面與剪貼簿',
    protocols: ['vnc'],
    hint: '遠端桌面的大小由 Mac 或 VNC 伺服器決定，瀏覽器會自動縮放顯示。單邊超過 4096 像素的螢幕需先調低解析度。',
    fields: [
      {
        name: 'color-depth',
        label: '色彩深度',
        type: 'select',
        options: [
          ['', '全彩（24 位元，建議）'],
          ['16', '16 位元（較省頻寬）'],
          ['8', '8 位元（頻寬最低）'],
        ],
      },
      { name: 'swap-red-blue', label: '對調紅藍色（顏色偏藍或偏紅時使用）', type: 'checkbox' },
      {
        name: 'clipboard-encoding',
        label: '剪貼簿文字編碼',
        type: 'select',
        options: [
          ['', 'ISO 8859-1（標準，僅西歐字元）'],
          ['UTF-8', 'UTF-8（macOS 螢幕共享的中文剪貼簿請選這個）'],
        ],
      },
    ],
  },
  {
    id: 'access',
    title: '連線控制',
    protocols: ALL,
    fields: [
      { name: 'timeout', label: '連線逾時（秒）', type: 'number', min: 1, max: 120, placeholder: '15' },
      { name: 'read-only', label: '唯讀：不傳送鍵盤與滑鼠輸入', type: 'checkbox' },
      { name: 'disable-copy', label: '停用從遠端複製到本機', type: 'checkbox' },
      { name: 'disable-paste', label: '停用從本機貼上到遠端', type: 'checkbox' },
    ],
  },
];
