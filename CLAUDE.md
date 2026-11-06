# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

共用工作準則以根目錄 [AGENTS.md](AGENTS.md) 為準，先讀該檔。本檔補充文件地圖與 daemon 相容性背景；使用者最新明確需求優先於舊計畫及 C 版筆記。

## 專案現況

cracamole 是不使用 Java 的瀏覽器 SSH／Telnet 系統，採 Rust Web 服務與純 JavaScript 前端。使用者已確認第一版採單一管理員，提供登入、連線新增／編輯／刪除、本機保存及終端機。現有 9 個 crate；`cracamole-web` 是完整系統入口，內嵌私人 loopback daemon。舊版「保留官方 Java webapp」範圍已被使用者取消，不能恢復為部署依賴。完整 MVP 仍待長時間 fuzz、資源／安全及 arm64 等驗收；不要把本機測試全綠當成全部完成。見 `docs/java-free-mvp.md` 與 `docs/MVP-STATUS.md`。

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

## C 版參考原始碼

`reference/guacamole-server/` 是 Apache `guacamole-server` 的本機 clone(`.gitignore` 已忽略,**不屬於本 repo**,不可 commit、不可把其程式碼複製進 Rust crate)。版本為 `main` @ `bb1ba8ed`(2026-11-03,`configure.ac` 為 1.6.1,最新兩個修正即 GUACAMOLE-2323 / 2350,與筆記一致)。上游沒有 1.6.1 tag,所以筆記是依 `main`。用途:回填 `⟳C` 預期值、核對筆記與檔名。唯讀使用。

## 文件地圖(動工前先讀)

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

- **MVP 範圍**：單一管理員登入、連線 CRUD、本機 JSON 保存及瀏覽器 SSH／Telnet。Rust 服務直接提供頁面與 WebSocket，不需要 Java、Tomcat、PostgreSQL 或 Node.js runtime。RDP、VNC、K8s、Serial、錄影、共享連線、SFTP、多使用者及內建 TLS 不在第一版。
- **Web 安全與保存**：Argon2id 驗證、HttpOnly/SameSite session、同源檢查、登入限流；登出與到期會關閉該 session 的 tunnel。管理員密碼由環境變數提供，至少 12 字元。連線機密保存在私有 volume 的 0600 JSON，尚未加密，API 只回傳是否已設定；不可將這些值放進日誌、URL 或 Git。
- **Greenfield**,只保證 Guacamole 線路協定(1.5.0)與連線參數名稱相容;不做 C/Rust 混編、不做 C ABI、不支援動態 `.so` 外掛(協定以 Cargo feature 靜態編入)。
- **單進程 tokio、每連線一個 task**(D2);外掛只能經 `SessionCtx` 與外界互動,不可碰 socket,以便日後改為 worker 進程隔離。
- **Crate 依賴單向**:`guac-protocol ← guac-session ← guac-display ← guac-terminal ← proto-* ← guacd ← cracamole-web`。`guac-protocol` 必須無 I/O、無 async、不得 panic。`guac-testkit` 只能是 dev-dependency。`web/` 資產內嵌於 Web binary，`web/vendor/` 的 JavaScript 保留 Apache LICENSE／NOTICE。
- **終端機實作**：`terminal.rs` 使用 `alacritty_terminal`／`vte` 與 `guac-display`，`font.rs` 使用內建字型與系統 fallback，不用 Pango。舊計畫的 `Grid`／`Renderer` 分層為規劃，不能當作現有 API。
- **Docker 完整系統**：`docker/compose.yml` 與 `docker/cracamole-web.Dockerfile` 為主要入口，只發布 HTTP。獨立 `guacd` 映像與 CLI 相容性測試仍保留，不能把它們誤當完整 Web 部署。`docker/compose.e2e.yml` 僅含 Rust 應用與 sshd／telnetd 測試主機。

## 工作規則

- **測試即規格**:行為先轉成案例 ID,登記於 `docs/TRACEABILITY.md`;與 C 版刻意不同處登記於 `docs/DEVIATIONS.md`。以 `python scripts/validation_report.py` 更新測試證據索引；未對應的規劃 ID 不能自動算通過。
- 案例預期值標 `⟳C` 者,表示須以運行中的 C 版 guacd 實測回填,**不可憑記憶當作事實**。
- 舊計畫的 crate 候選與成熟度判斷不代表現況。以 Cargo.toml／Cargo.lock、實際 API、授權檔與驗證證據為準；新增依賴須核對授權與適用性，不可把「已編譯」當作完整審核。
- C 版既有缺陷(見 `00-index` §2.6)預設：可觀察行為相容，安全缺陷與明顯 bug 不照抄並記入 DEVIATIONS。現行 SSH 未設定 host key 時略過驗證並記 warn；有設定但無法解析或比對失敗時不得降級為不驗證。此行為不是已完成所有安全驗收的宣告。
