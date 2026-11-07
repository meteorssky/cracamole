# 專案工作準則

本檔適用整個 repository；文件與工作回覆使用繁體中文。先讀本檔及 `README.md`，再依任務讀實際程式碼。使用者最新的明確需求優先於舊計畫；不得以舊 C 系統的設計推翻產品範圍。

## 現行產品範圍

- 整套系統不使用 Java。伺服器為 Rust，瀏覽器使用 HTML／CSS／JavaScript；不得恢復 Java webapp、Tomcat 或 Maven 作為建置、部署或 E2E 依賴。
- 第一版採單一管理員，提供登入／登出、連線新增／編輯／刪除、本機保存與瀏覽器 SSH／Telnet。使用者已要求加入 RDP（`proto-rdp`，IronRDP 引擎），開發中，範圍見 README；多使用者、角色、VNC、SFTP、錄影與共享連線延後。
- 完整系統入口為 `cracamole-web`，內嵌私人 loopback daemon；`web/` 靜態資產編入 binary。正式執行不需要 Node.js 或外部資料庫；Node.js 僅供瀏覽器測試工具使用。
- 正式部署使用 `docker/compose.yml`、`docker/cracamole-web.Dockerfile`。`guacd` 獨立映像與 C oracle 僅保留相容性用途，不代表完整 Web 系統。

## 架構與資料

- Workspace 共九個 crate；維持依賴方向，底層不得依賴 Web 層，`guac-testkit` 僅作 dev-dependency。
- 管理員帳密由環境變數提供，沒有正式預設密碼。沿用 Argon2id、HttpOnly／SameSite cookie、同源檢查與登入限流；登出、session 到期與關機須關閉相關 tunnel。
- 連線設定保存於私有 JSON／Docker volume。機密目前未做靜態加密，Unix 目錄／檔案權限為 0700／0600；不得宣稱已加密。API 不回傳密碼、私鑰或密語；不得寫入 URL、日誌或 Git。編輯時未提供的機密保留，明確空值才清除。
- daemon 僅監聽私人 loopback，不發布 4822。正式 Compose 預設僅發布主機 loopback HTTP；遠端使用透過 HTTPS reverse proxy，保留 Host 與 WebSocket，啟用 Secure cookie。
- `web/vendor/` 的 Apache JavaScript 保留 LICENSE／NOTICE；更新時使用釘選來源與 checksum。

## 驗證與回歸

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

依改動範圍執行必要檢查，純文件修改不必重跑產品測試。Docker／瀏覽器流程見 README；E2E 只使用 Rust 應用與 sshd／telnetd fixture。涉及顯示、tunnel 或關機的修改須保留以下驗證：

- 驗證瀏覽器合成後的實際終端機截圖，不能只讀 canvas backing buffer；維持終端機 stacking context，避免負 z-index canvas 被背景遮蔽。
- tunnel 關閉須更新前端狀態；正常 WebSocket 關閉送 1000。關閉測試不可把任意錯誤／EOF 當作正常關閉。
- 活躍 SSH 的 SIGTERM／SIGINT 必須在時限內 exit 0，並讓頁面顯示連線結束。
- 保存終端機最小格點、尺寸／字型配置上限、共享字型快取、單一 CJK fallback 與 idle heartbeat 等現有防護，不可為了讓測試通過而移除。
- Docker 使用持久 Cargo target cache 時，必須確保來源更新會重新編譯，避免舊 mtime 導致映像帶入過期 binary。

測試通過數、ignored／缺少環境而跳過、實際執行的平台與尚未驗收項目須分別說明。已有 CI 設定不代表已執行；單一管理員第一版通過也不代表長時間 fuzz、arm64 及全部 MVP 硬化門檻完成。

## 參考來源與環境

- `reference/guacamole-server/` 唯讀、不屬於本 repo；不可修改、commit 或直接複製 C 程式碼進 Rust crate。C 筆記僅作相容性事實參考，與實測不符時更正；安全缺陷不照抄。
- 現行規格與證據見本機 `docs/java-free-mvp.md`、`docs/java-free-validation-report.md`、`docs/MVP-STATUS.md`。`docs/`、`reference/` 依既有設定被忽略，不得自行變更追蹤偏好；重要啟動規則亦須保留在可追蹤的 README／本檔。
- 遠端驗證先檢查現有程序、容器與資源；測試設記憶體／PID 上限。完成後保存證據，只清理本輪建立的容器、網路、volume 與 forwarding，不影響其他服務。
- 變更範圍或部署入口時，同步更新 README、本檔、CLAUDE.md 及適用計畫；不要留下互相矛盾的準則。
