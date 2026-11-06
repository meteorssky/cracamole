"""Generate test evidence without treating absent cases as passing."""
import argparse
import re
from pathlib import Path


def read_log(path):
    data = path.read_bytes()
    return data.decode("utf-16" if data.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-log", type=Path, default=Path("target/mvp-tests.log"))
    parser.add_argument("--stress-log", type=Path, default=Path("target/mvp-stress.log"))
    parser.add_argument("--output", type=Path, default=Path("docs/TRACEABILITY.md"))
    options = parser.parse_args()
    results = {}
    for path in [options.test_log, options.stress_log]:
        if path.exists():
            results.update(re.findall(r"^test (\w+) \.\.\. (ok|FAILED|ignored)", read_log(path), re.M))

    lines = [
        "# MVP 測試證據索引", "",
        "由 `python scripts/validation_report.py` 依本機測試輸出產生。", "",
        "`ok` 只代表該 Rust 測試通過；部分測試在缺少參考碼、ssh-keygen、sh 或 CJK 字型時會提早返回。",
        "此索引不能證明 Docker、瀏覽器 E2E、C 版差分、vttest 或長時間 fuzz 已驗收。",
        "規劃 ID 與測試名稱相同前綴僅供查找，仍須審查斷言是否涵蓋整條規格。", "",
        "| 測試檔 | 函式 | 本機結果 |", "|---|---|---|",
    ]
    names = []
    for path in sorted(Path("crates").glob("*/tests/*.rs")):
        source = path.read_text(encoding="utf-8")
        for match in re.finditer(r"#\[(?:tokio::)?test[^\]]*\](?:\s*#\[[^\]]*\])*\s*(?:async\s+)?fn\s+(\w+)", source):
            name = match[1]
            names.append(name)
            relative = path.as_posix()
            lines.append(f"| [{relative}](../{relative}) | `{name}` | {results.get(name, '未執行')} |")

    plan = Path("docs/mvp-test-plan.md")
    if plan.exists():
        text = plan.read_text(encoding="utf-8")
        ids = set(re.findall(r"\b[A-Z]{2,}(?:-[A-Z]+)?-\d{2}[a-z]?\b", text))
        for prefix1, first, prefix2, last in re.findall(r"([A-Z]+(?:-[A-Z]+)?)-(\d{2})[~～–]([A-Z]+(?:-[A-Z]+)?)-(\d{2})", text):
            if prefix1 == prefix2:
                ids.update(f"{prefix1}-{i:02}" for i in range(int(first), int(last) + 1))
        lines += ["", "## 規劃 ID 查找", "", "無同名測試者保留待對應；不能以總測試數代替退出條件。", "",
                  "| 規劃 ID | 同前綴測試 |", "|---|---|"]
        for case in sorted(ids):
            prefix = case.lower().replace("-", "_")
            matches = [name for name in names if name == prefix or name.startswith(prefix + "_")]
            lines.append(f"| {case} | {'、'.join('`' + n + '`' for n in matches) or '待對應／待補測'} |")
    options.output.parent.mkdir(parents=True, exist_ok=True)
    options.output.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"{options.output}: {len(names)} Rust test functions indexed")


if __name__ == "__main__":
    main()
