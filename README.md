# cracamole

以 Rust 重寫 Apache Guacamole 的 `guacd`。目前已實作 Guacamole 握手、終端機、SSH 與 Telnet；Docker 與官方 Java webapp 的整體驗收仍待執行，尚未達成 MVP 全部退出條件。

```sh
cargo build --locked -p guacd
cargo run -p guacd -- -f -b 127.0.0.1 -l 4822 -L info
```

驗證指令：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --workspace --locked --test stress -- --ignored --nocapture
cargo bench -p guac-terminal --bench mvp --locked
python -m unittest discover -s tests/oracle -v
```

Windows 若遇中斷建置留下的增量連結錯誤，可在 PowerShell 先執行 `$env:CARGO_INCREMENTAL='0'` 再重試。測試用 `ssh-keygen` 產生金鑰、`sh` 驗證 entrypoint；CJK 渲染需要系統中文字型，或以 `CRACAMOLE_FONT_DIRS` 指定字型目錄。缺少這些資源時，相關測試目前會跳過。

Docker 建置：

```sh
docker build -f docker/cracamole.Dockerfile -t cracamole:local .
```

將既有 Guacamole Compose 中的 guacd 映像改為 `cracamole:local`，或套用 [Compose overlay](docker/compose.example.yml)。沿用既有的資料庫、認證、webapp 與後端網路。daemon 支援 `-f -b -l -L`；容器支援 `LOG_LEVEL`，並相容舊名 `GUACD_LOG_LEVEL`。4822 沒有 TLS 或客戶端認證，僅限受信任的內部網路。

完整測試環境（只使用一次性測試帳密）：

```sh
docker compose -p cracamole-e2e -f docker/compose.e2e.yml up -d --build --wait
cd tests/e2e
npm ci
npx playwright install chromium
npm test
```

webapp 位於 `http://127.0.0.1:18080/guacamole/`，帳密為 `guacadmin` / `guacadmin`。測試經官方登入頁面連線到真實 sshd/telnetd，驗證鍵盤、vim 儲存、視窗縮放與中文輸出，並留存截圖；此測試環境尚未在本機啟動驗證。結束後從 repo 根目錄執行：

```sh
docker compose -p cracamole-e2e -f docker/compose.e2e.yml down -v
```

C 版 `args` 差分：

```sh
docker compose -p cracamole-oracle -f docker/compose.oracle.yml up -d --build --wait
python tests/oracle/compare_args.py
docker compose -p cracamole-oracle -f docker/compose.oracle.yml down -v
```

結果寫入 `target/oracle/args.json`，包含 C/C 自比對以及 SSH、Telnet 的 C/Rust 原始位元組比對。預設 oracle 是官方 1.6.0 映像，與本機 `main@bb1ba8ed` 參考碼版本不同；發佈驗收須指定並保存實際映像 digest（`C_GUACD_IMAGE` 可覆寫）。環境配置依據 [Apache Docker 文件](https://guacamole.apache.org/doc/1.6.0/gug/guacamole-docker.html) 與 [PostgreSQL 文件](https://guacamole.apache.org/doc/1.6.0/gug/postgresql-auth.html)。

原始碼分成 8 個 crate：`guac-protocol`、`guac-session`、`guac-display`、`guac-terminal`、`proto-telnet`、`proto-ssh`、`guacd`、`guac-testkit`。`fuzz/` 是獨立工作區；用法見 [fuzz README](fuzz/README.md)。CI 包含 Windows/Linux 測試、Docker、C 版參數差分、瀏覽器 E2E，以及每個 fuzz target 一小時的排程設定；尚未取得 CI 執行結果。

MVP 不含 RDP、VNC、Kubernetes、Serial、SFTP、錄影、共享連線及 TLS。完整計畫、偏離記錄及驗收狀態位於本機 `docs/`（目前依既有 `.gitignore` 設定不追蹤）。
