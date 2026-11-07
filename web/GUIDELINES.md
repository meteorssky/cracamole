# 前端開發準則

適用範圍：`web/` 下所有檔案（`web/vendor/` 的第三方程式除外，不得修改）。
設計背景與路線見 `docs/ux-redesign.md`（本機文件）。**本檔的規則用「必須／不得」表示強制，用「應」表示預設、需要理由才能偏離。**

---

## 1. 不可違反的限制

1. **無建置步驟、無 Node runtime**：正式執行只靠 Rust 執行檔內嵌的靜態檔。不得引入打包器、轉譯器、框架或 npm 執行時依賴。Node 僅用於測試（`tests/e2e`）。
2. **CSP 為 `script-src 'self'`**：不得使用行內 `<script>`、`onclick=` 屬性、`eval`、`new Function`。樣式允許行內（`style-src 'unsafe-inline'`），但**應**用 class 與 token，不寫行內樣式。
3. **`web/vendor/` 唯讀**：保留 Apache LICENSE／NOTICE；不修補。需要行為調整時，在自己的適配層處理。
4. **機密不得出現在**：URL、`localStorage`／`sessionStorage`、日誌、`console.*`、DOM 屬性、錯誤訊息。密碼、私鑰、密語只存在於表單欄位與送出的請求本文；API 只回傳「是否已設定」。
5. **不得向第三方網域發出請求**（字型、CDN、分析、圖示皆需自帶）。
6. **內容語言為繁體中文**，用語依 §8。

---

## 2. 檔案結構

```
web/
  index.html            # 只放骨架與地標；不放業務邏輯
  css/
    tokens.css          # 唯一的設計 token 來源（顏色／間距／字級／圓角／陰影／動效）
    base.css            # reset、排版、焦點、減少動態、forced-colors
    layout.css          # 版面（登入、頂欄、連線庫抽屜、舞台、狀態列）
    components/*.css    # 每個元件一檔：button、form、badge、card、dialog、overlay、toast、keybar、banner
  js/
    main.js             # 進入點：只做組裝與啟動
    api.js              # 唯一的 fetch 封裝（錯誤正規化 {status,message,field}、逾時、401 統一處理）
    state.js            # 單一事件化狀態
    session/            # guac.js（唯一接觸 Guacamole 的地方）、reasons.js（結束原因→可行動訊息）、keys.js（輔助鍵）
    views/              # login、drawer、library、editor、stage
    ui/                 # dialog.js（唯一的對話框／確認機制）、toast.js
    util/               # dom.js、protocols.js、theme.js
  vendor/               # 第三方（唯讀）
  GUIDELINES.md
```
- 一個檔案一個職責；**超過 250 行必須拆分**。
- 新增靜態檔必須列入 `crates/cracamole-web/src/assets.rs` 的資產表（單一表、單一 `/assets/{*path}` 路由）。`every_web_file_is_served` 測試會在漏列時失敗。
- 資產目前以 `Cache-Control: no-store` 提供；加入內容雜湊與快取屬於效能預算工作（見 §11）。

---

## 3. CSS 規則

1. **所有數值走 token**：顏色、間距、字級、圓角、陰影、動效時間只能用 `var(--…)`。禁止在元件檔寫死 `#hex`、`px` 間距（1px 邊框、0 除外）。
2. **語意命名**：`--surface`、`--text-muted`、`--danger`；禁止 `--green-2`、`--gray-700` 這類值名稱用於元件。
3. **命名**：BEM 風格 `.card`、`.card__title`、`.card--active`；狀態用 `aria-*` 或 `data-state` 選擇器（`[aria-selected="true"]`），不用 `.is-xxx` 混雜。
4. **不得使用 `!important`**（唯一例外：`[hidden]{display:none!important}` 與 `prefers-reduced-motion` 覆寫）。
5. **選擇器深度 ≤ 2**；不得用 ID 作樣式選擇器。
6. **邏輯屬性優先**（`margin-inline`、`padding-block`、`inset`），為日後 RTL／直書預留。
7. **單位**：字級與間距用 `rem`；觸控目標、邊框用 `px`。**文字不得小於 14px**（等寬終端機字型除外）。CJK 文字**不得**設定負的 `letter-spacing`。
8. **必須處理的媒體查詢**：`prefers-color-scheme`、`prefers-reduced-motion`、`forced-colors`；斷點只有 `--bp-sm: 760px` 一個（行動優先，往大寫）。
9. **對比**：正文 ≥ 4.5:1、大字與 UI 邊框 ≥ 3:1；**以腳本計算 token 對，不憑目測**（新增或修改顏色時必須附上計算結果）。
10. **焦點**：每個可聚焦元素都要有 `:focus-visible` 樣式（2px 輪廓＋2px 偏移，對比 ≥ 3:1）；**不得** `outline:none` 而不補替代。
11. **動畫**：只動 `transform`／`opacity`；150–200ms；尊重 `prefers-reduced-motion`。

---

## 4. JavaScript 規則

1. **原生 ES module**（`<script type="module">`）；不使用全域變數；每個模組明確 `export`。
2. **嚴格模式與語法**：`const` 優先、`let` 其次、不用 `var`；`===`；不用隱式型別轉換。
3. **格式**：2 空白縮排、單引號、行寬 ≤ 100、結尾分號、一行一個語句（**禁止把多個語句塞成一行**，現有 `app.js` 的長行屬待遷移債務）。以專案 `.editorconfig`／Prettier 設定統一（僅開發工具，不進入正式執行）。
4. **DOM 安全**：插入使用者可控內容（連線名稱、主機名稱、遠端剪貼簿、錯誤訊息）**只能**用 `textContent`、`setAttribute`、`replaceChildren`。**禁止**對動態資料使用 `innerHTML`、`insertAdjacentHTML`、`outerHTML`。
5. **網路**：所有請求經 `api.js`。它負責：同源、`credentials: same-origin`、逾時、把失敗正規化為 `{status, code, message, field?}`；401 統一導向登入並關閉工作階段。元件不得直接呼叫 `fetch`。
6. **狀態**：單一 `state` 物件＋明確的事件（`state.on('connections', …)`）。元件**不得**彼此直接修改對方的 DOM；透過狀態或事件溝通。
7. **Guacamole 適配層**（`js/session/`）：全專案只有此處可以 `new Guacamole.*`。對外提供 `connect(connection, el) → Session`，`Session` 暴露：`state`（`connecting|connected|reconnecting|closed`）、`closeReason`（`remote-exit|network|auth|timeout|expired|server`）、`resize()`、`sendKeys()`、`clipboard`、`dispose()`。UI 不依賴 Guacamole 的原始碼值。
8. **資源回收**：每個 `addEventListener`、`ResizeObserver`、計時器、Guacamole client 都必須有對應的釋放；工作階段關閉時**必須**呼叫 `dispose()`。不得留下游離的 `blur`／`keydown` 監聽。
9. **錯誤處理**：不得吞錯（空的 `catch {}` 禁止）。使用者可見的錯誤必須 (a) 說明發生什麼、(b) 指出可做的動作、(c) 就地顯示（欄位旁或覆蓋層），toast 只能是補充。
10. **鍵盤**：終端機／遠端桌面取得焦點時轉送按鍵；必須提供「離開終端機」的鍵盤出口（避免焦點陷阱）；不得無條件 `preventDefault` 整個頁面的快捷鍵。
11. **可測試性**：每個互動元素有穩定的可及名稱（`aria-label` 或可見文字），供測試以 role／name 取用。**測試不得依賴 CSS class 或 DOM 結構**。

---

## 5. 元件規則

- **一個概念一個元件**：對話框、確認、通知、覆蓋層、分頁各只有一種實作；不得新增第二種「確認」機制（`window.confirm`／`alert`／`prompt` **禁止使用**）。
- 每個元件**必須**實作並測試狀態：預設、hover、focus-visible、active、disabled、loading、error（適用者）。
- **對話框**用原生 `<dialog>`：開啟時聚焦第一個互動元素或標題；`Esc` 關閉；關閉後焦點回到觸發者；背景不可操作。
- **按鈕**：每個視圖**最多一個**主要按鈕；危險動作用危險樣式且需二次確認；載入中 `aria-busy` 並停用，防止重複送出。
- **表單**：標籤永遠可見（placeholder 不是標籤）；錯誤用 `aria-describedby` 連結、`aria-invalid="true"`；送出時就地顯示欄位級錯誤並聚焦第一個錯誤欄位。
- **狀態不只靠顏色**：狀態點必須搭配文字或圖示。
- **圖示**：內嵌 SVG（`currentColor`）；純圖示按鈕必須有 `aria-label`。

---

## 5.1 操作效率與終端機體驗準則（Speed-of-Use）

1. **畫面最大化與可收折工具列（Stage Maximization & Collapsible Chrome）**：
   - **終端機／RDP 舞台優先**：預設頂欄與狀態列高度應保持緊湊；連線進行中必須提供**「最大畫面（收折選單）」**與**「全螢幕」**切換，讓 SSH／RDP 畫面可隨時擴展至 **100% 視窗面積（`100vw × 100dvh`）**。
   - **連線庫抽屜覆蓋不擠壓**：當有活躍連線時，展開左側連線庫抽屜必須以**浮動覆蓋層（overlay）**呈現，**不得**推擠舞台寬度而觸發遠端 PTY／RDP 非預期的重新縮放（resize）；抽屜須支援一鍵收折與 `Esc` 關閉。
2. **連線狀態常駐可見（Persistent Connection Status）**：
   - 無論選單處於展開或收折（最大畫面）狀態，使用者都必須能一眼確認當前連線狀態（未連線、正在連線、已連線、連線已結束、連線失敗）與連線時間。
   - 工具列收折時，右上角須保留輕量狀態膠囊（含狀態燈號、主機名稱、狀態文字與「展開選單」按鈕），且位置不得遮擋終端機左上角輸入區。
   - 連線庫中的活躍卡片須同步顯示即時連線狀態燈號與文字，並將最近連線的主機置頂、其餘依名稱穩定排序（禁止直接暴露隨機 UUID 順序）。
3. **過多設定的漸進式揭露（Progressive Disclosure for Settings）**：
   - 編輯對話框必須採用**固定標題列與固定底部操作列（Sticky Header & Footer）**，僅中間表單區捲動，確保「取消／儲存連線／儲存並連線」永遠可見。
   - 基本連線欄位採雙欄緊湊排列；進階設定依協定分組收折於 `<details>` 中，且每個折疊標題（`<summary>`）須即時顯示**「已設定 N 項／預設」摘要徽章**，並提供「全部收合」控制，避免設定項目過多造成捲動迷航。
4. **最短進入終端機路徑與鍵盤逃生口**：
   - 新增／編輯表單應同時提供「儲存連線」與「儲存並連線」；搜尋框按 `/` 聚焦、按 `Enter` 直接開啟第一筆符合的主機。
   - 終端機攔截按鍵時，**必須**保留 `Ctrl+Shift+Escape` 作為離開終端機焦點（並展開選單）的鍵盤逃生口，不得造成絕對焦點陷阱。

---

## 6. 安全相關的 UX 規則

1. 密碼／私鑰欄位：`autocomplete="new-password"`（連線機密）或 `current-password`（登入）；編輯時**不回填**，顯示「已設定」徽章與「變更」。
2. 純 HTTP（非 localhost）必須顯示固定警告「連線未加密」。
3. SSH 首次連線需顯示主機金鑰指紋並由使用者決定；指紋改變必須阻擋並以危險樣式說明。**不得靜默降級為不驗證**。
4. RDP「接受自簽憑證」需二次確認並說明風險；預設為嚴格驗證。
5. 工作階段到期前預告並允許延長；到期後清空畫面上的敏感內容並導向登入。
6. 遠端剪貼簿內容視為不可信：只當文字顯示，上限大小，不解析為 HTML。

---

## 7. 無障礙清單（每個 PR 必勾）

- [ ] 只用鍵盤可完成此次變更涉及的所有功能；Tab 順序合理；沒有焦點陷阱。
- [ ] 所有互動元素有可及名稱；地標與標題階層正確（一頁一個 `h1`）。
- [ ] 對比達標（附計算值）；狀態不只靠顏色；沒有小於 14px 的文字。
- [ ] 觸控目標：手機 ≥ 44×44px、桌機 ≥ 24×24px。
- [ ] 200% 縮放與 320px 寬無水平捲動（終端機區除外）。
- [ ] `prefers-reduced-motion`、`prefers-color-scheme`、`forced-colors` 皆已檢視。
- [ ] axe 掃描零嚴重／重大違規。
- [ ] 至少一次人工鍵盤走查；涉及動態內容時做一次讀屏走查，結果寫在 PR。

---

## 8. 文案規則（繁體中文）

- **台灣用語**：「連線」「伺服器」「登入／登出」「設定」「儲存」「刪除」「剪貼簿」「主機」；避免「鏈接」「服務器」「登錄」。
- **動詞開頭、具體**：「新增連線」「儲存變更」；不用「確定」「是／否」當按鈕文字（用「刪除這台主機」「取消」）。
- **錯誤訊息三段式**：發生什麼 → 可能原因 → 下一步。例：「無法連線到 10.0.1.5:22。主機沒有回應，可能已關機或被防火牆擋住。請確認位址後按『重新連線』。」不得只寫「失敗」「參數無效」。
- **不責備使用者**；不使用驚嘆號；不使用行銷語氣。
- **半形標點用於數字／英文與技術字**（`SSH`、`10.0.1.5:22`），中文句子用全形標點；中英文之間不強制加空格，但**同一檔案內須一致**。
- 專有名詞固定：SSH、Telnet、RDP、NLA、TLS、host key（中文「主機金鑰」）。

---

## 9. 測試與驗收

1. **每個 UI 行為對應一個 Playwright 案例**（`tests/e2e/*.spec.js`），在 Chromium、Firefox、WebKit 全部通過。以 `scripts/e2e_remote.sh <host>` 執行完整套件；RDP 快速迴圈用 `scripts/rdp_dev.sh <host>`。
2. **選擇器**只用 `getByRole`、`getByLabel`、`getByText`；不用 CSS class、nth-child、結構路徑。
3. **畫面斷言必須看畫素**：終端機／遠端桌面不能只斷言「狀態是已連線」，要斷言畫面確實有內容、局部更新後內容仍在（這是曾經漏掉的真實缺陷）。
4. **無障礙**：`@axe-core/playwright` 掃描登入、工作區、每個對話框，零嚴重／重大違規。
5. **視覺回歸**：登入、空狀態、連線庫、編輯表單、覆蓋層，各在 1280px 與 390px、亮與暗色截圖；差異需人工核准。
6. **對比腳本**：token 前景／背景對比值低於門檻即測試失敗。
7. **效能**：登入頁傳輸 ≤ 60KB（不含 `guacamole.js`）、CLS = 0、按鍵到畫面 p95 ≤ 100ms（區網）；回歸 > 20% 失敗。
8. **失敗路徑也要測**：斷線、驗證失敗、限流、到期、後端回錯誤，皆要有案例，並驗證畫面上有可行動的訊息（不是空白）。

---

## 10. 完成定義（PR 清單）

- [ ] 符合 §1 的所有限制（無新增建置步驟、無行內腳本、無第三方請求、機密未外洩）。
- [ ] CSS 只用 token；沒有 `!important`、寫死顏色、小於 14px 文字。
- [ ] JS 為模組；沒有 `innerHTML` 接動態資料、沒有直接 `fetch`、沒有 `confirm/alert/prompt`；資源皆已釋放。
- [ ] 元件的所有狀態都已實作並有測試。
- [ ] 錯誤訊息符合 §8 三段式且就地顯示。
- [ ] §7 無障礙清單全勾；axe 零違規。
- [ ] Playwright 三個瀏覽器通過；新增行為有案例；畫素斷言到位。
- [ ] 390px 與 1280px、亮與暗色截圖已附上並核准。
- [ ] 效能預算達標（附數字）。
- [ ] `cargo test --workspace --locked`、`clippy -D warnings` 仍為綠（前端變更不得破壞後端）。
- [ ] 文件（README／本準則）有需要更新者已更新。

---

## 11. 遷移債務（狀態）

| 項目 | 狀態 |
|---|---|
| `app.js` 單檔、多語句長行 | **已完成**：拆成 §2 的模組 |
| `confirm()` 刪除確認 | **已完成**：統一對話框 `confirmAction`（預設在「取消」） |
| 直接 `fetch`／`Guacamole.*` 散落 | **已完成**：收斂到 `api.js` 與 `session/guac.js` |
| 淺色殼＋純黑終端機 | **已完成**：token 化、亮暗兩套（預設暗色，可切換並記住）；終端機表面有專屬 `--on-terminal*` token |
| `--muted` 對比 3.85–4.15:1、10px 標籤 | **已完成**：所有 token 對由 `tests/e2e/contrast.test.js` 驗證（88 項） |
| 資產逐檔路由 | **已完成**：資產表＋防漏測試 |
| 內容雜湊與快取 | 未做（目前 `no-store`）；列入效能預算工作 |
| `guacamole.js` 延後載入 | 未做；目前在頁面載入時載入 |
| SSH 主機金鑰首次信任 | **已完成**：首次阻擋，顯示指紋，核對後保存；變更／撤銷時阻擋 |
| 終端機鍵盤焦點逃生口（`Ctrl+Shift+Escape`） | 未做（目前 `guac.js` 無條件攔截所有按鍵，違反 §4.10 與 §5.1） |
| 行動端實機軟鍵盤喚出（隱藏 `<textarea>` 代理） | 未做（目前 `#terminal` 為 `div[tabindex=0]`，實機點擊無法喚出系統虛擬鍵盤） |
| 連線庫排序（最近使用／名稱）與搜尋 `Enter` 直連 | 未做（目前依後端 UUID 字典序排列，搜尋後無法按 `Enter` 直接連線） |
| 連線中展開抽屜不觸發遠端 PTY／RDP resize | 未做（目前桌機 `.drawer` 為 flex 推擠，展開即縮小舞台寬度 320px） |
| 「儲存並連線」、智慧貼上 `user@host:port`、複製連線 | 未做（新增主機後需回清單尋找卡片再點一次） |
| 切換主機／重整頁面的防誤斷保護 | 未做（點擊另一張卡片或頂欄品牌連結會無預警中斷當前連線） |
| 多分頁工作階段、命令面板、編輯器內建測試連線 | 未做（P1，部分需要後端 API） |
