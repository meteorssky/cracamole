"""Exercise the built image on a Linux Docker host, including three active sessions at shutdown."""
import argparse
import codecs
import json
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests" / "oracle"))
from compare_args import instruction


def docker(*args):
    return subprocess.check_output(["docker", *args], text=True, stderr=subprocess.STDOUT).strip()


def encode(*elements):
    return (",".join(f"{len(element)}.{element}" for element in elements) + ";").encode()


class Client:
    def __init__(self, host, port):
        self.socket = socket.create_connection((host, port), timeout=30)
        self.decoder = codecs.getincrementaldecoder("utf-8")()
        self.buffer = ""
        self.disconnected = False
        self.failure = None

    def recv(self):
        while True:
            result = instruction(self.buffer)
            if result:
                elements, used = result
                self.buffer = self.buffer[used:]
                return elements
            chunk = self.socket.recv(16384)
            if not chunk:
                raise EOFError("connection closed without disconnect")
            self.buffer += self.decoder.decode(chunk)

    def drain(self):
        try:
            while True:
                message = self.recv()
                if message[0] == "sync":
                    self.socket.sendall(encode("sync", message[1]))
                elif message[0] == "disconnect":
                    self.disconnected = True
                    return
                elif message[0] == "error":
                    raise RuntimeError(str(message))
        except Exception as error:
            self.failure = str(error)


def run(image, signal):
    name = f"cracamole-validation-{signal.lower()}"
    created = False
    clients = []
    workers = []
    upstreams = []
    first_frames = []
    listener = None
    started = time.monotonic()
    try:
        docker("run", "-d", "--name", name, "--read-only", "--cap-drop", "ALL",
               "--memory", "1g", "--pids-limit", "64",
               "--security-opt", "no-new-privileges", "--health-interval", "1s",
               "-e", "LOG_LEVEL=debug", image)
        created = True
        deadline = time.monotonic() + 40
        while True:
            state = json.loads(docker("inspect", name))[0]
            if state["State"]["Health"]["Status"] == "healthy":
                break
            if not state["State"]["Running"] or time.monotonic() > deadline:
                raise RuntimeError(docker("logs", name))
            time.sleep(0.2)
        startup = time.monotonic() - started
        assert docker("exec", name, "id", "-u") == "1000"
        assert docker("exec", name, "cat", "/proc/1/comm") == "guacd"
        assert state["HostConfig"]["ReadonlyRootfs"]
        assert state["HostConfig"]["CapDrop"] == ["ALL"]
        assert state["HostConfig"]["Memory"] == 1024 ** 3
        assert not state["NetworkSettings"]["Ports"].get("4822/tcp")
        assert "protocols:" in docker("logs", name), "LOG_LEVEL=debug did not take effect"
        network = next(iter(state["NetworkSettings"]["Networks"].values()))

        # Bind the fixture to Docker's bridge, so it is reachable only through that interface.
        listener = socket.socket()
        listener.bind((network["Gateway"], 0))
        listener.listen(3)
        listener.settimeout(10)
        backend_port = listener.getsockname()[1]
        for _ in range(3):
            first_frame_start = time.monotonic()
            client = Client(network["IPAddress"], 4822)
            clients.append(client)
            client.socket.sendall(encode("select", "telnet"))
            args = client.recv()
            assert args[0] == "args"
            params = {"hostname": network["Gateway"], "port": str(backend_port)}
            client.socket.sendall(encode("size", "640", "480", "96"))
            client.socket.sendall(encode("audio"))
            client.socket.sendall(encode("video"))
            client.socket.sendall(encode("image", "image/png"))
            client.socket.sendall(encode("connect", args[1], *(params.get(key, "") for key in args[2:])))
            assert client.recv()[0] == "ready"
            upstream, _ = listener.accept()
            upstreams.append(upstream)
            upstream.sendall(b"Container validation\r\n$ ")
            # Receiving a complete frame proves a live backend and renderer are active.
            while True:
                message = client.recv()
                assert message[0] != "error", message
                if message[0] == "sync":
                    client.socket.sendall(encode("sync", message[1]))
                    break
            first_frames.append(round(time.monotonic() - first_frame_start, 3))
            client.socket.settimeout(10)
            worker = threading.Thread(target=client.drain, daemon=True)
            worker.start()
            workers.append(worker)

        memory = json.loads(docker("stats", "--no-stream", "--format", "{{json .}}", name))
        start_stop = time.monotonic()
        docker("kill", "--signal", signal, name)
        code = docker("wait", name)
        elapsed = time.monotonic() - start_stop
        for worker in workers:
            worker.join(timeout=1)
        assert code == "0", f"exit code {code}"
        assert elapsed < 10, f"shutdown took {elapsed:.2f}s"
        assert all(client.disconnected for client in clients), [client.failure for client in clients]
        return {"signal": signal, "healthy": True, "uid": 1000, "pid1": "guacd",
                "read_only": True, "cap_drop": "ALL", "published_ports": False,
                "memory_limit_bytes": 1024 ** 3,
                "memory_sample": memory["MemUsage"], "first_frame_seconds": first_frames,
                "startup_seconds": round(startup, 3), "active_sessions": 3,
                "client_disconnects": 3, "exit_code": 0, "shutdown_seconds": round(elapsed, 3)}
    finally:
        for client in clients:
            client.socket.close()
        for upstream in upstreams:
            upstream.close()
        if listener:
            listener.close()
        if created:
            docker("rm", "-f", name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", default="cracamole:ci")
    parser.add_argument("--output", type=Path, default=Path("target/validation/container.json"))
    args = parser.parse_args()
    deprecated = docker("run", "--rm", "-e", "GUACD_LOG_LEVEL=warning", args.image, "-v")
    assert "deprecated" in deprecated and "Guacamole proxy daemon" in deprecated
    precedence = docker("run", "--rm", "-e", "LOG_LEVEL=info", "-e",
                        "GUACD_LOG_LEVEL=invalid", args.image, "-v")
    assert "Guacamole proxy daemon" in precedence and "deprecated" not in precedence
    docker("run", "--rm", "--entrypoint", "sh", args.image, "-c",
           "! command -v cargo && ! command -v rustc && test ! -d /src")
    print("entrypoint variables and runtime contents: PASS", flush=True)
    records = []
    for signal in ["SIGTERM", "SIGINT"]:
        record = run(args.image, signal)
        records.append(record)
        print(json.dumps(record), flush=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(records, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
