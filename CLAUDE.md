# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

共用工作準則以根目錄 [AGENTS.md](AGENTS.md) 為準，先讀該檔。本檔補充文件地圖與 daemon 相容性背景；使用者最新明確需求優先於舊計畫及 C 版筆記。

## 專案現況

cracamole 是不使用 Java 的遠端連線系統，採 Rust Web 服務與純 JavaScript 前端，Windows 原生介面開發中。第一版採單一管理員，提供登入、連線新增／編輯／刪除、本機保存及 SSH／Telnet／RDP。現有 13 個 crate（含 `proto-vnc`），`cracamole-store` 共用帳密驗證與連線設定；`cracamole-desktop` 提供基本連線編輯、指紋核對與預覽，使用獨立私有資料目錄。TLS 與 NLA 模式已與獨立 loopback RDP 測試伺服器驗證工作階段、影格與輸入；Windows x64 可見桌面已比對縮放前後各九個 RDP 測試影格像素及 SSH 測試文字與背景。Windows x64 互動測試已驗證系統剪貼簿 Unicode 文字讀寫；真實 Windows／xrdp 服務及其 NLA 互通性、遠端剪貼簿交換、完整鍵鼠相容性及更廣泛的可見畫面尚未驗收。`cracamole-web` 是完整 Web 系統入口，內嵌私人 loopback daemon。舊版「保留官方 Java webapp」範圍已被使用者取消，不能恢復為部署依賴。完整 MVP 仍待長時間 fuzz、資源／安全及 arm64 等驗收；不要把本機測試全綠當成全部完成。見 `docs/java-free-mvp.md` 與 `docs/MVP-STATUS.md`。

Windows 桌面預設入口為現代化的 GPUI 版（獨立 workspace `desktop-gpui/`，`scripts/run_desktop.ps1` 或 `cargo run -p cracamole-desktop` 啟動器）；Win32 原生版保留為備援，執行檔為 `cracamole-desktop-win32`，既有原生自動化測試皆針對它。GPUI 版以 39 項無頭測試及離屏 wgpu 渲染驗證，實機畫面、高 DPI、實體輸入與系統剪貼簿尚待使用者驗收；其測試需設 `RUST_MIN_STACK=536870912`。

Windows 桌面版一次性 SSH 可在核對指紋後，將目的地與公鑰獨立保存在私有資料目錄的 `ssh-identities.json`，不保存連線或帳密；信任衝突不得自動取代。清除方式與限制見 README。

常用指令：

```sh
cargo build --locked -p guacd
cargo build --locked -p cracamole-web
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --workspace --locked --test stress -- --ignored --nocapture
cargo bench -p guac-terminal --bench mvp --locked
```

`fuzz/` 為獨立 workspace。Docker 與瀏覽器測試用法見 README。本機 Windows 若遇增量連結錯誤，先設定 `$env:CARGO_INCREMENTAL='0'` 再試。部分測試會在缺少參考碼、ssh-keygen、sh 或 CJK 字型時提早返回，檢查結果時須留意。`docs/` 依現有 `.gitignore` 仍被忽略；不要自行變更使用者的追蹤偏好。

文件以繁體中文撰寫,請沿用。

終端機編碼：SSH／Telnet 預設 UTF-8，`character-encoding` 可明確選擇 Big5、GBK、GB18030 與日韓等舊編碼；僅轉換遠端終端機文字。SSH 驗證與 Web／Guacamole 仍用 Unicode，RDP 剪貼簿維持 Unicode；不得正規化密碼。跨封包解碼、不可表示輸入的整批拒絕及真實瀏覽器字形比較見 README 與 `scripts/text_dev.sh`。C oracle 須明確傳入 `--allow-terminal-encoding-extension`，不能把前綴不符當作相容。

## C 版參考原始碼

`reference/guacamole-server/` 是 Apache `guacamole-server` 的本機 clone(`.gitignore` 已忽略,**不屬於本 repo**,不可 commit、不可把其程式碼複製進 Rust crate)。版本為 `main` @ `bb1ba8ed`(2026-11-03,`configure.ac` 為 1.6.1,最新兩個修正即 GUACAMOLE-2323 / 2350,與筆記一致)。上游沒有 1.6.1 tag,所以筆記是依 `main`。用途:回填 `⟳C` 預期值、核對筆記與檔名。唯讀使用。

## 文件地圖(動工前先讀)

- `docs/reference-gap-analysis.md` — **C 參考實作 vs 現有功能差距全盤掃描報告**（2026-11-08 最新落實現狀對照，涵蓋架構、協定、17 領域、安全與 UI/UX 比對）。
- `docs/rust-migration/00-index-and-coverage.md` — C 版相容性參考入口，含已被證偽的前提與已知缺陷。僅在判讀歷史 C 行為時參考較新的 addendum；現行產品範圍以使用者決定與 `AGENTS.md` 為準，實際行為須核對程式碼及測試。
- `docs/rust-migration/NN-*.md` — 對原 C 專案(guacamole-server 1.6.1)的逐檔閱讀筆記，作為 daemon 相容性參考。不能覆蓋使用者明確指定的無 Java 與單一管理員範圍。引用格式 `[11]` = `11-protocol-ssh.md`。覆蓋率約 93% 且為推論，與 C 版實測不符時應更正筆記。
- `docs/java-free-mvp.md` — 現行完整系統架構、檔案、API、帳號範圍、資料保存與無 Java 驗收；優先於舊計畫中保留 Java webapp 的內容。
- `docs/java-free-validation-report.md` — 無 Java 系統實際驗證結果：318 個 workspace 測試、Web 修正後七案例、五個瀏覽器案例與 Docker 隔離／保存權限檢查；不要將之擴大為全部 MVP 退出條件通過。
- `docs/rust-migration-assessment.md` — 第一輪(抽樣)評估;部分結論已過時(見該檔頂端更正)。
- `docs/mvp-development-plan.md` — MVP 範圍、架構決策 D1–D9、里程碑 M0–M5、Docker 部署(§10)、開放問題 Q1–Q7。
- `docs/mvp-file-architecture.md` — 規劃中的 Cargo workspace 逐檔職責,以及 C 檔 → Rust 檔對照。
- `docs/mvp-test-plan.md` — 測試層級 T0–T10、進出條件、案例 ID 目錄。
- `docs/libguac`、`docs/libguac-terminal` — 僅含 Doxyfile,與功能無關。

## 架構要點(跨文件才看得出來)

- **MVP 範圍**：單一管理員登入、連線 CRUD、本機 JSON 保存及瀏覽器 SSH／Telnet。Rust 服務直接提供頁面與 WebSocket，不需要 Java、Tomcat、PostgreSQL 或 Node.js runtime。RDP 為使用者後續明確要求加入（`crates/proto-rdp`，以 IronRDP 為引擎；已支援畫面、鍵盤、滑鼠、文字剪貼簿與動態縮放，音訊、磁碟機／檔案重新導向與 RemoteApp 尚未完成；`scripts/rdp_dev.sh` 為遠端 xrdp＋本機編譯的快速測試迴圈）；VNC 為使用者後續明確要求加入（`crates/proto-vnc`，自製 RFB 客戶端，無第三方 VNC crate；以連線 macOS 螢幕共享為優先：ARD 驗證類型 30、VNC 密碼類型 2；編碼 Raw／CopyRect／Hextile／ZRLE；`scripts/vnc_dev.sh` 於遠端 Docker 主機跑 TigerVNC 容器互通測試，`scripts/vnc_vectors.py` 為 ARD／DES 的獨立已知答案向量；尚未在真實 macOS 上驗證，Tight／Cursor 偽編碼／客戶端發起的縮放不支援）；K8s、Serial、錄影、共享連線、SFTP、多使用者及內建 TLS 不在第一版。
- **Web 安全與保存**：Argon2id 驗證、HttpOnly/SameSite session、同源檢查、登入限流；登出與到期會關閉該 session 的 tunnel。管理員密碼由環境變數提供，至少 12 字元。連線機密保存在私有 volume 的 0600 JSON，尚未加密，API 只回傳是否已設定；不可將這些值放進日誌、URL 或 Git。
- **Greenfield**,只保證 Guacamole 線路協定(1.5.0)與連線參數名稱相容;不做 C/Rust 混編、不做 C ABI、不支援動態 `.so` 外掛(協定以 Cargo feature 靜態編入)。
- **協定描述自己**：`ProtocolPlugin::info()` 提供顯示名稱、預設連接埠與可核對的伺服器身分（`IdentityKind`：host-key／certificate），`arg_names()` 即該協定接受的參數。Web 經 `GET /api/protocols` 取得，前端的協定選單、預設埠、參數過濾與信任流程都由它驅動；新增協定不需修改 web 層的協定名單。仍屬協定專屬的只有各協定的表單群組（`web/index.html` 的 `data-protocols`）、`option-sections.js` 的進階設定、RDP 的 NLA 說明與 Ctrl+Alt+Del 鈕，以及 `store.rs` 的 RDP `ignore-cert` 規則。
- **單進程 tokio、每連線一個 task**(D2);外掛只能經 `SessionCtx` 與外界互動,不可碰 socket,以便日後改為 worker 進程隔離。
- **Crate 依賴單向**:`guac-protocol ← guac-session ← guac-display ← guac-terminal ← proto-*（含 proto-vnc，僅依賴 guac-session／guac-display／guac-protocol）← guacd ← cracamole-web`。`guac-protocol` 必須無 I/O、無 async、不得 panic。`guac-testkit` 只能是 dev-dependency。`web/` 資產內嵌於 Web binary，`web/vendor/` 的 JavaScript 保留 Apache LICENSE／NOTICE。
- **終端機實作**：`terminal.rs` 使用 `alacritty_terminal`／`vte` 與 `guac-display`，`font.rs` 使用內建字型與系統 fallback，不用 Pango。舊計畫的 `Grid`／`Renderer` 分層為規劃，不能當作現有 API。
- **Docker 完整系統**：`docker/compose.yml` 與 `docker/cracamole-web.Dockerfile` 為主要入口，只發布 HTTP。獨立 `guacd` 映像與 CLI 相容性測試仍保留，不能把它們誤當完整 Web 部署。`docker/compose.e2e.yml` 僅含 Rust 應用與 sshd／telnetd 測試主機。

## 工作規則

- **測試即規格**:行為先轉成案例 ID,登記於 `docs/TRACEABILITY.md`;與 C 版刻意不同處登記於 `docs/DEVIATIONS.md`。以 `python scripts/validation_report.py` 更新測試證據索引；未對應的規劃 ID 不能自動算通過。
- 案例預期值標 `⟳C` 者,表示須以運行中的 C 版 guacd 實測回填,**不可憑記憶當作事實**。
- 舊計畫的 crate 候選與成熟度判斷不代表現況。以 Cargo.toml／Cargo.lock、實際 API、授權檔與驗證證據為準；新增依賴須核對授權與適用性，不可把「已編譯」當作完整審核。
- C 版既有缺陷(見 `00-index` §2.6)預設：可觀察行為相容，安全缺陷與明顯 bug 不照抄並記入 DEVIATIONS。SSH 未設定 host key 時拒絕認證，由 Web 顯示指紋供使用者核對並保存信任；撤銷優先，無效設定不得降級為不驗證。RDP 遠端尺寸必須在配置前驗證並取得共用記憶體額度；剪貼簿有總 bytes、串流數與閒置上限，貼上控制碼須過濾。
- 代理限流：只信任 `CRACAMOLE_TRUSTED_PROXIES` 列出的 TCP peer 所覆寫的單一 `X-Real-IP`，不信任任意 `X-Forwarded-For`。SSH／RDP 身分探測共用併發上限、完整 deadline 及 session／shutdown 取消；細節見 README。這些防護不是全部安全驗收已完成的宣告。
