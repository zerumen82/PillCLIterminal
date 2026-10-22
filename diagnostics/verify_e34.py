"""Controles locales reproducibles; guarda evidencia sin instalar dependencias."""
import hashlib
import json
import pathlib
import subprocess
import sys
import time


ROOT = pathlib.Path(__file__).resolve().parents[1]
REPORT = ROOT / "evidencias/2026-10-20-e34-checks.json"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    report = {"checks": [], "result": "FAIL"}
    commands = [
        ["cargo", "fmt", "--all", "--", "--check"],
        ["cargo", "clippy", "--locked", "--offline", "--all-targets", "--", "-D", "warnings"],
        ["cargo", "test", "--locked", "--offline"],
        ["cargo", "build", "--release", "--locked", "--offline"],
    ]
    for command in commands:
        started = time.monotonic()
        result = subprocess.run(command, cwd=ROOT, capture_output=True, encoding="utf-8", errors="replace")
        report["checks"].append({"command": command, "exit_code": result.returncode,
                                 "elapsed_s": time.monotonic() - started,
                                 "stdout": result.stdout, "stderr": result.stderr})
        REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print("PASS" if result.returncode == 0 else "FAIL", " ".join(command), flush=True)
        if result.returncode:
            print(result.stderr, flush=True)
            return result.returncode
    metadata = subprocess.run(["cargo", "metadata", "--locked", "--offline", "--filter-platform",
                               "x86_64-pc-windows-msvc", "--format-version", "1"], cwd=ROOT,
                              capture_output=True, encoding="utf-8", errors="replace")
    report["metadata_exit_code"] = metadata.returncode
    if metadata.returncode:
        report["metadata_error"] = metadata.stderr
    else:
        packages = json.loads(metadata.stdout)["packages"]
        report["packages"] = [{"name": p["name"], "version": p["version"], "license": p["license"]}
                              for p in packages if p["source"] is not None]
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "config/defaults.json"]
    paths += sorted((ROOT / "src").rglob("*.rs")) + sorted((ROOT / "tests").rglob("*.rs"))
    report["source_sha256"] = {str(path.relative_to(ROOT)): digest(path) for path in paths}
    report["binary_sha256"] = digest(ROOT / "target/release/pill.exe")
    report["integration_note"] = "Las pruebas ignoradas no cuentan como PASS de integracion; informes opt-in separados."
    report["result"] = "PASS" if metadata.returncode == 0 else "FAIL"
    REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    return metadata.returncode


if __name__ == "__main__":
    sys.exit(main())
