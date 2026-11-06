"""Vendor the Apache Guacamole JavaScript client without Java, Maven, or npm."""
import hashlib
from pathlib import Path
import tarfile
import urllib.request

URL = "https://codeload.github.com/apache/guacamole-client/tar.gz/refs/tags/1.6.0"
SHA256 = "de5c489471544f93dfc0cc821cf95805aaf9b84e2e3314b6d52e41aeeffd7c16"
archive = Path("target/guacamole-client-1.6.0.tar.gz")
archive.parent.mkdir(exist_ok=True)
if not archive.exists():
    urllib.request.urlretrieve(URL, archive)
if hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
    raise SystemExit("Upstream archive checksum mismatch")
destination = Path("web/vendor")
destination.mkdir(parents=True, exist_ok=True)
with tarfile.open(archive) as source:
    modules = [m for m in source.getmembers() if "/src/main/webapp/modules/" in m.name and m.name.endswith(".js")]
    modules.sort(key=lambda m: (not m.name.endswith("/Namespace.js"), m.name))
    bundle = b"// Apache Guacamole common-js 1.6.0; see NOTICE and LICENSE.\n"
    for module in modules:
        bundle += b"\n// " + module.name.rsplit("/", 1)[-1].encode() + b"\n" + source.extractfile(module).read() + b"\n"
    (destination / "guacamole.js").write_bytes(bundle)
    for name in ["NOTICE", "LICENSE"]:
        (destination / name).write_bytes(source.extractfile("guacamole-client-1.6.0/" + name).read())
print(f"Vendored {len(modules)} JavaScript modules; no Java build required")
