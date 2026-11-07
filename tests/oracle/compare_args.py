"""Compare live C/Rust args instructions byte-for-byte. Python standard library only."""
import argparse
import json
import socket
from pathlib import Path


def instruction(text):
    """Return (elements, consumed characters), or None for a partial instruction."""
    elements = []
    pos = 0
    while True:
        dot = text.find(".", pos)
        if dot < 0:
            return None
        length = text[pos:dot]
        if not length.isascii() or not length.isdecimal():
            raise ValueError("invalid length")
        count = int(length)
        if count > 8192 or len(elements) >= 128:
            raise ValueError("protocol limit exceeded")
        end = dot + 1 + count
        if len(text) <= end:
            return None
        elements.append(text[dot + 1:end])
        terminator = text[end]
        pos = end + 1
        if terminator == ";":
            return elements, pos
        if terminator != ",":
            raise ValueError("invalid terminator")


def args_at(host, port, protocol):
    with socket.create_connection((host, port), timeout=10) as sock:
        sock.sendall(f"6.select,{len(protocol)}.{protocol};".encode())
        wire = bytearray()
        while len(wire) < 4 * 8192 * 128:
            chunk = sock.recv(8192)
            if not chunk:
                raise RuntimeError("EOF before args")
            wire.extend(chunk)
            try:
                text = wire.decode("utf-8")
            except UnicodeDecodeError as exc:
                if exc.reason == "unexpected end of data":
                    continue
                raise
            result = instruction(text)
            if result is not None:
                elements, consumed = result
                if elements[0] != "args":
                    raise RuntimeError(f"expected args, received {elements[0]}")
                return text[:consumed].encode("utf-8")
        raise RuntimeError("response exceeded protocol limits")


def accepted_args(oracle, candidate, allow_terminal_encoding=False):
    """Keep raw equality meaningful; accept only the documented appended extension when requested."""
    extension = b",18.character-encoding;"
    return oracle == candidate or (allow_terminal_encoding and candidate == oracle[:-1] + extension)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle-host", default="127.0.0.1")
    parser.add_argument("--oracle-port", type=int, default=14822)
    parser.add_argument("--candidate-host", default="127.0.0.1")
    parser.add_argument("--candidate-port", type=int, default=24822)
    parser.add_argument("--output", type=Path, default=Path("target/oracle/args.json"))
    parser.add_argument("--allow-terminal-encoding-extension", action="store_true")
    options = parser.parse_args()
    records = {}
    for protocol in ["ssh", "telnet"]:
        oracle = args_at(options.oracle_host, options.oracle_port, protocol)
        # C vs C self-comparison is required before trusting this recording.
        repeat = args_at(options.oracle_host, options.oracle_port, protocol)
        candidate = args_at(options.candidate_host, options.candidate_port, protocol)
        records[protocol] = {
            "oracle": oracle.decode(),
            "oracle_repeat": repeat.decode(),
            "candidate": candidate.decode(),
            "stable": oracle == repeat,
            "equal": oracle == candidate,
            "accepted": accepted_args(oracle, candidate, options.allow_terminal_encoding_extension),
        }
        oracle_args = instruction(oracle.decode())[0][2:]
        candidate_args = instruction(candidate.decode())[0][2:]
        records[protocol]["candidate_only_args"] = [arg for arg in candidate_args if arg not in oracle_args]
        records[protocol]["oracle_only_args"] = [arg for arg in oracle_args if arg not in candidate_args]
    options.output.parent.mkdir(parents=True, exist_ok=True)
    options.output.write_text(json.dumps(records, indent=2), encoding="utf-8")
    for protocol, record in records.items():
        print(f"{protocol}: stable={record['stable']} equal={record['equal']} accepted={record['accepted']}")
        if not record["equal"]:
            print(f"  candidate-only: {record['candidate_only_args']}; oracle-only: {record['oracle_only_args']}")
    if not all(r["stable"] and r["accepted"] for r in records.values()):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
