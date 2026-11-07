# cracamole

以 Rust 實作的瀏覽器 SSH／Telnet 系統。第一版提供單一管理員登入、連線新增／編輯／刪除、本機保存設定及瀏覽器終端機。伺服器不需要 Java、Tomcat、Maven 或資料庫服務；瀏覽器使用 JavaScript。

Rust Web 服務內含終端機 daemon，透過僅限 loopback 的私人連線交換 Guacamole 協定。對外只提供 HTTP／WebSocket。

## 啟動

在 repo 根目錄建立 `.env`（已被 Git 忽略），設定自己的管理員密碼，至少 12 個字元：

```dotenv
CRACAMOLE_ADMIN_USERNAME=admin
CRACAMOLE_ADMIN_PASSWORD=replace-with-your-own-long-password
```

```sh
docker compose -f docker/compose.yml up -d --build --wait
```

開啟 <http://localhost:8080/>，使用上述帳密登入，再新增 SSH 或 Telnet 連線。目的地主機名稱由容器解析；`localhost` 指容器本身。Docker 的 `connections` volume 保存連線設定；`down -v` 會刪除此資料。

管理員帳密由環境變數提供，session 儲存在記憶體，重啟後須重新登入。連線密碼與私鑰以明文保存在權限 `0600` 的 `/data/connections.json`，目錄權限 `0700`；API 不回傳這些機密。請保護 volume、備份及 `.env`。編輯時密碼留空會保留原值，勾選清除才會移除。

SSH 進階設定可填入已知的主機公鑰以驗證伺服器；留空沿用目前 daemon 的不驗證政策。

遠端使用時，在前方配置支援 WebSocket 的 HTTPS reverse proxy，保持相同 Host 與 Origin，並設定 `CRACAMOLE_SECURE_COOKIE=true`。應用程式目前不自行終止 TLS。預設 Compose 只將 8080 發布到 `127.0.0.1`，沒有發布 daemon 連接埠。正式容器以 UID 1000、唯讀根檔案系統及移除全部 capabilities 執行。

本機開發（PowerShell）：

```powershell
$env:CRACAMOLE_ADMIN_PASSWORD='replace-with-your-own-long-password'
cargo run --locked -p cracamole-web
```

預設 <http://127.0.0.1:8080/>，資料位於被忽略的 `data/`。可設定 `CRACAMOLE_HTTP_BIND`、`CRACAMOLE_DATA_DIR`、`CRACAMOLE_ADMIN_USERNAME`、`CRACAMOLE_SECURE_COOKIE`、`LOG_LEVEL`。

## 驗證

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --workspace --locked --test stress -- --ignored --nocapture
cargo bench -p guac-terminal --bench mvp --locked
python -m unittest discover -s tests/oracle -v
```

瀏覽器 E2E 使用 Rust 應用程式及一次性的 sshd／telnetd 測試容器，全程沒有 Java：

```sh
docker compose -p cracamole-e2e -f docker/compose.e2e.yml up -d --build --wait
cd tests/e2e
npm ci
npx playwright install chromium
npm test
cd ../..
python scripts/validate_web_container.py
```

測試站台為 <http://127.0.0.1:18080/>，測試專用帳密 `admin` / `fixture-admin-password-2026`。五個案例涵蓋 SSH／Telnet 鍵盤輸入、vim 儲存、PTY 縮放、剪貼簿、中文、閒置連線、登出、連線編輯／重啟保存／刪除，以及活躍 SSH 連線的 SIGTERM／SIGINT。畫面檢查使用瀏覽器實際截圖，避免 canvas 被背景遮住卻空過。完成後從 repo 根目錄執行：

```sh
docker compose -p cracamole-e2e -f docker/compose.e2e.yml down -v
```

遠端 Docker 可搭配本機 Chromium。在另一個終端保持 SSH forwarding，再設定測試環境變數：

```powershell
ssh -N -L 127.0.0.1:18080:127.0.0.1:18080 user@docker-host
$env:CRACAMOLE_E2E_SSH='user@docker-host'
$env:CRACAMOLE_E2E_REMOTE_DIR='/path/to/cracamole'
cd tests/e2e
npm test
```

更簡單的做法是一個指令（本機不需要 Docker）：`scripts/e2e_remote.sh user@docker-host [playwright 參數]`。它會把工作目錄（含未提交變更）同步到遠端的獨立目錄、以獨立的 compose 專案啟動 Rust 應用與 sshd／telnetd、建立 SSH 轉發，於 Chromium、Firefox 與 WebKit 執行整套 Playwright，結束後自動清理，不影響遠端已部署的服務。

前端的快速檢查不需要任何主機：`cd tests/e2e && node --test contrast.test.js syntax.test.js`（色彩對比與 ES module 語法）。介面行為與無障礙（axe）案例在 `tests/e2e/ui.spec.js`，可用 `scripts/rdp_dev.sh user@docker-host -g "ui:"` 在本機編譯的程式上執行。前端規範見 `web/GUIDELINES.md`。

RDP 的快速開發迴圈：`scripts/rdp_dev.sh user@docker-host`。只在遠端啟動 xrdp 測試容器、建立 SSH 轉發，並使用本機編譯的 `cracamole-web` 執行 RDP 的 Playwright 案例（略過約 40 分鐘的完整映像建置）。

Windows 若遇中斷建置留下的增量連結錯誤，可先設定 `$env:CARGO_INCREMENTAL='0'`。部分既有測試需要 `ssh-keygen`、`sh`、參考 C 原始碼或 CJK 字型；缺少時會跳過，檢視結果時須留意。

## 原始碼與相容性測試

Workspace 有 10 個 crate：`guac-protocol`、`guac-session`、`guac-display`、`guac-terminal`、`proto-telnet`、`proto-ssh`、`proto-rdp`、`guacd`、`guac-testkit`、`cracamole-web`。`web/` 是前端靜態資產，直接內嵌於 Rust 執行檔；正式執行不需要 Node.js。Apache Guacamole 的 JavaScript 客戶端附於 `web/vendor/`，保留授權與 NOTICE，可用 `python scripts/vendor_guacamole.py` 重建。

`guacd` 仍可獨立執行並驗證線路相容性；它不是完整系統的部署入口。C 版 oracle 僅用於測試，預設釘在上游 `bb1ba8ed1414ec6a239cea265d4e9c7e23b9c6a3`，不屬於正式執行環境：

```sh
docker compose -p cracamole-oracle -f docker/compose.oracle.yml up -d --build --wait
python tests/oracle/compare_args.py
docker compose -p cracamole-oracle -f docker/compose.oracle.yml down -v
```

`fuzz/` 是獨立工作區，見 [fuzz README](fuzz/README.md)。CI 包含 Rust 檢查、Docker、C args 差分及無 Java 瀏覽器 E2E；設定 CI 不等於已取得執行證據。長時間 fuzz、arm64 與完整效能／安全驗收仍須另行完成。

RDP 以 `proto-rdp`（IronRDP）提供畫面、鍵盤、滑鼠、文字剪貼簿（雙向）與動態縮放（瀏覽器視窗改變大小，遠端桌面跟著調整）；音訊、磁碟機／檔案重新導向與 RemoteApp 尚未完成。目前不含多使用者、VNC、Kubernetes、Serial、SFTP、錄影及共享連線。詳細計畫與驗證記錄位於本機 `docs/`，依既有設定不追蹤。
