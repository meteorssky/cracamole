"""After browser E2E, verify the Java-free image, isolation, and saved data permissions."""
import json
from pathlib import Path
import subprocess


def docker(*args):
    return subprocess.check_output(["docker", *args], text=True).strip()


container = docker("compose", "-p", "cracamole-e2e", "-f", "docker/compose.e2e.yml", "ps", "-q", "web")
assert container, "Web test container is missing"
info = json.loads(docker("inspect", container))[0]
assert info["State"]["Running"]
assert docker("exec", container, "id", "-u") == "1000"
assert docker("exec", container, "cat", "/proc/1/comm") == "cracamole-web"
assert info["HostConfig"]["ReadonlyRootfs"]
assert info["HostConfig"]["CapDrop"] == ["ALL"]
assert info["HostConfig"]["Memory"] == 1536 * 1024 * 1024
assert "4822/tcp" not in info["NetworkSettings"]["Ports"]
docker("exec", container, "sh", "-c", "! command -v java && ! command -v javac && ! command -v psql && ! command -v cargo && ! command -v rustc")
listeners = []
for row in docker("exec", container, "cat", "/proc/net/tcp").splitlines()[1:]:
    columns = row.split()
    if columns[3] == "0A":
        host, port = columns[1].split(":")
        listeners.append((host, int(port, 16)))
assert any(host == "0100007F" and port != 8080 for host, port in listeners), listeners
assert all(host.endswith("7F") or port == 8080 for host, port in listeners), listeners
docker("exec", container, "sh", "-c", "set -eu; test $(stat -c %a /data) = 700; test $(stat -c %a /data/connections.json) = 600")
record = {"java_runtime": False, "postgresql_client": False, "uid": 1000, "pid1": "cracamole-web",
          "read_only": True, "cap_drop": "ALL", "private_daemon": True, "published_backend": False,
          "memory_limit_bytes": info["HostConfig"]["Memory"], "image": info["Image"],
          "data_directory_mode": "0700", "connection_file_mode": "0600"}
output = Path("target/validation/java-free-container.json")
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
print(json.dumps(record))
