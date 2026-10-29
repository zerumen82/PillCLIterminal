"""Ejecuta integraciones opt-in en serie; conserva cada salida y fallo."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
CASES = [
    ('ollama-chat', 'ollama_live', 'real_chat_and_errors'),
    ('ollama-cancel', 'ollama_live', 'real_cancel_and_retry'),
    ('ollama-errors', 'ollama_live', 'real_connection_and_context_errors'),
    ('chat-tools', 'tools_live', 'model_repairs_project_with_tools'),
    ('five-agents', 'tools_live', 'five_agents_repair_and_verify_project'),
    ('stream-cut', 'stream_live', 'real_tool_stream_interruption_never_executes_unconfirmed_calls'),
    ('coding-corpus', 'coding_corpus', 'real_model_repairs_two_multi_file_tasks_preserving_bom_crlf_and_backups'),
]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--model', required=True)
    parser.add_argument('--python', required=True)
    parser.add_argument('--num-ctx', type=int, required=True)
    parser.add_argument('--num-predict', type=int, required=True)
    parser.add_argument('--think', action=argparse.BooleanOptionalAction, default=False)
    parser.add_argument('--evidence-prefix', type=Path, required=True)
    args = parser.parse_args()
    prefix = args.evidence_prefix.resolve()
    prefix.parent.mkdir(parents=True, exist_ok=True)
    artifacts = [prefix.with_name(prefix.name + '-' + label + suffix)
                 for label, _, _ in CASES for suffix in ('.json', '.stdout.log', '.stderr.log')]
    summary = prefix.with_name(prefix.name + '-summary.json')
    if summary.exists() or any(path.exists() for path in artifacts):
        raise FileExistsError('Elige otro prefijo; no se sobrescriben ejecuciones anteriores.')
    paths = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'config/defaults.json',
             *sorted((ROOT / 'src').rglob('*.rs')), *sorted((ROOT / 'tests').glob('*.rs'))]
    hashes = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}
    report = {'fecha_utc': datetime.now(timezone.utc).isoformat(), 'model': args.model,
              'source_sha256': hashes, 'checks': [], 'result': 'IN_PROGRESS',
              'scope': 'Ollama y herramientas reales; cada caso explícito en serie, sin instalar ni descargar modelos.'}
    child_env = dict(os.environ, PILL_LIVE_MODEL=args.model, PILL_TEST_PYTHON=args.python,
                     PILL_LIVE_CONTEXT=str(args.num_ctx), PILL_LIVE_PREDICT=str(args.num_predict),
                     PILL_LIVE_THINK=str(args.think).lower())
    for label, suite, case in CASES:
        evidence = prefix.with_name(prefix.name + '-' + label + '.json')
        stdout = prefix.with_name(prefix.name + '-' + label + '.stdout.log')
        stderr = prefix.with_name(prefix.name + '-' + label + '.stderr.log')
        command = ['cargo', 'test', '--locked', '--offline', '--test', suite, case,
                   '--', '--ignored', '--exact', '--nocapture']
        child_env['PILL_LIVE_EVIDENCE'] = str(evidence)
        started = time.monotonic()
        print('RUN ' + label, flush=True)
        with stdout.open('wb') as out, stderr.open('wb') as err:
            completed = subprocess.run(command, cwd=ROOT, env=child_env, stdin=subprocess.DEVNULL,
                                       stdout=out, stderr=err)
        check = {'case': label, 'command': command, 'exit_code': completed.returncode,
                 'elapsed_s': time.monotonic() - started, 'evidence': str(evidence),
                 'stdout': str(stdout), 'stderr': str(stderr),
                 'pass': completed.returncode == 0 and evidence.is_file()}
        report['checks'].append(check)
        summary.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
        print(('PASS ' if check['pass'] else 'FAIL ') + label, flush=True)
    report['sources_unchanged'] = all(hashlib.sha256((ROOT / path).read_bytes()).hexdigest() == digest
                                     for path, digest in hashes.items())
    report['result'] = 'PASS' if report['sources_unchanged'] and all(c['pass'] for c in report['checks']) else 'FAIL'
    summary.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    sys.exit(main())
