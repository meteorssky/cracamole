# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 專案現況

cracamole 是 Apache Guacamole 伺服器端(`guacd` 與各協定外掛)的 **Rust 重寫**。**目前 repo 只有文件,沒有任何程式碼**(無 `Cargo.toml`、無 C 原始碼)。因此尚無 build / lint / test 指令可用;不要臆測或虛構這些指令。

文件以繁體中文撰寫,請沿用。

## 文件地圖(動工前先讀)

- `docs/rust-migration/00-index-and-coverage.md` — **入口**。C 版系統的事實、已被證偽的前提、已知缺陷清單。與其他文件衝突時,以 `rust-migration/` 為準(編號較大 / addendum 者優先)。
- `docs/rust-migration/NN-*.md` — 對原 C 專案(guacamole-server 1.6.1)的逐檔閱讀筆記,是**唯一的規格來源**,原始碼不在此 repo。引用格式 `[11]` = `11-protocol-ssh.md`。覆蓋率約 93% 且為推論,與 C 版實測不符時應更正筆記。
- `docs/rust-migration-assessment.md` — 第一輪(抽樣)評估;部分結論已過時(見該檔頂端更正)。
- `docs/mvp-development-plan.md` — MVP 範圍、架構決策 D1–D9、里程碑 M0–M5、Docker 部署(§10)、開放問題 Q1–Q7。
- `docs/mvp-file-architecture.md` — 規劃中的 Cargo workspace 逐檔職責,以及 C 檔 → Rust 檔對照。
- `docs/mvp-test-plan.md` — 測試層級 T0–T10、進出條件、案例 ID 目錄。
- `docs/libguac`、`docs/libguac-terminal` — 僅含 Doxyfile,與功能無關。

## 架構要點(跨文件才看得出來)

- **MVP 範圍**:用未修改的官方 Java webapp,以 cracamole 取代 guacd,可登入 SSH / Telnet 並使用 vim、htop。RDP、VNC、K8s、Serial、錄影、共享連線、SFTP、TLS **明確不在 MVP 內**。新增範圍須同時刪除一項。
- **Greenfield**,只保證 Guacamole 線路協定(1.5.0)與連線參數名稱相容;不做 C/Rust 混編、不做 C ABI、不支援動態 `.so` 外掛(協定以 Cargo feature 靜態編入)。
- **單進程 tokio、每連線一個 task**(D2);外掛只能經 `SessionCtx` 與外界互動,不可碰 socket,以便日後改為 worker 進程隔離。
- **Crate 依賴單向**:`guac-protocol ← guac-session ← guac-display ← guac-terminal ← proto-* ← guacd`。`guac-protocol` 必須無 I/O、無 async、不得 panic。`guac-testkit` 只能是 dev-dependency。
- **終端機三層**:格點引擎(`alacritty_terminal` / `vte`,待 PoC-1)→ `Grid` 抽象 → `Renderer` trait(內建字型光柵化,不用 Pango)。
- **Docker 取代官方 `guacamole/guacd`**:必須支援 `-f -b -L` 與 `LOG_LEVEL` / `GUACD_LOG_LEVEL`,所以 `-L` 是 MVP 必做。

## 工作規則

- **測試即規格**:行為先轉成案例 ID,登記於 `docs/TRACEABILITY.md`(尚未建立);與 C 版刻意不同處登記於 `docs/DEVIATIONS.md`(尚未建立)。
- 案例預期值標 `⟳C` 者,表示須以運行中的 C 版 guacd 實測回填,**不可憑記憶當作事實**。
- Rust crate 的成熟度與授權判斷(`alacritty_terminal`、`russh`、`cosmic-text`/`fontdue` 等)皆**未連網驗證**,需在 M0 PoC 確認。
- C 版既有缺陷(見 `00-index` §2.6)預設:可觀察行為相容,安全缺陷與明顯 bug 不照抄並記入 DEVIATIONS。SSH 未設 host key 時是否驗證尚待決定(Q2)。
