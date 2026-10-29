"""Servidor MCP stdio mínimo para las pruebas de pill. Solo biblioteca estándar."""

import json
import os
import sys
import time


def send(message):
    line = json.dumps(message, ensure_ascii=False)
    with open("outbox.txt", "a", encoding="utf-8") as record:
        record.write(line + "\n")
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def schema(mode):
    if mode == "badref":
        return {"$ref": "https://example.invalid/no-existe.json"}
    if mode == "badfile":
        return {"$ref": "file:///C:/Windows/win.ini"}
    return {
        "type": "object",
        "properties": {"text": {"type": "string"}},
        "required": ["text"],
        "additionalProperties": False,
    }


def tools(mode):
    return [
        {
            "name": "echo",
            "description": "Devuelve el texto recibido.",
            "inputSchema": schema(mode),
            "annotations": {"readOnlyHint": True},
        }
    ]


def main():
    sys.stdin.reconfigure(encoding="utf-8", newline="\n")
    sys.stdout.reconfigure(encoding="utf-8", newline="\n")
    mode = sys.argv[1] if len(sys.argv) > 1 else "echo"
    with open("started.txt", "w", encoding="utf-8") as marker:
        marker.write(mode)
    with open("launches.txt", "a", encoding="utf-8") as marker:
        marker.write(str(os.getpid()) + "\n")
    if mode == "descendant":
        import subprocess
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with open("descendant.txt", "w", encoding="utf-8") as marker:
            marker.write(str(child.pid))
    for raw in sys.stdin:
        with open("inbox.txt", "a", encoding="utf-8") as record:
            record.write(raw)
        line = raw.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "id" not in message:
            continue
        method = message.get("method", "")
        identifier = message["id"]
        if method == "initialize":
            params = message.get("params") or {}
            version = params.get("protocolVersion") or "2024-11-05"
            send(
                {
                    "jsonrpc": "2.0",
                    "id": identifier,
                    "result": {
                        "protocolVersion": version,
                        "capabilities": {"tools": {"listChanged": False}},
                        "serverInfo": {"name": "pill-fixture", "version": "0"},
                    },
                }
            )
        elif method == "ping":
            send({"jsonrpc": "2.0", "id": identifier, "result": {}})
        elif method == "tools/list":
            if mode == "hang":
                time.sleep(30)
                return
            send({"jsonrpc": "2.0", "id": identifier, "result": {"tools": tools(mode)}})
        elif method == "tools/call":
            if mode in ("write_die", "write_hang", "write_error"):
                with open("effect.txt", "w", encoding="utf-8") as effect:
                    effect.write("efecto real antes del resultado")
                if mode == "write_die":
                    os._exit(3)
                if mode == "write_hang":
                    time.sleep(30)
                    return
            if mode == "die":
                os._exit(3)
            if mode == "hang":
                time.sleep(30)
                return
            params = message.get("params") or {}
            text = (params.get("arguments") or {}).get("text", "")
            send(
                {
                    "jsonrpc": "2.0",
                    "id": identifier,
                    "result": {
                        "content": [{"type": "text", "text": text}],
                        "isError": mode == "write_error",
                    },
                }
            )
        else:
            send(
                {
                    "jsonrpc": "2.0",
                    "id": identifier,
                    "error": {"code": -32601, "message": method},
                }
            )


if __name__ == "__main__":
    main()
