"""Captura reproducible de pruebas E2; no instala ni libera recursos."""
import argparse
import ctypes as C
from ctypes import wintypes as W
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
K = C.WinDLL('kernel32', use_last_error=True)
P = C.WinDLL('psapi', use_last_error=True)


class ProcessEntry(C.Structure):
    _fields_ = [('size', W.DWORD), ('usage', W.DWORD), ('pid', W.DWORD),
                ('heap', C.c_size_t), ('module', W.DWORD), ('threads', W.DWORD),
                ('parent', W.DWORD), ('priority', W.LONG), ('flags', W.DWORD),
                ('name', W.WCHAR * 260)]


class Memory(C.Structure):
    _fields_ = [('size', W.DWORD), ('faults', W.DWORD)] + [
        (name, C.c_size_t) for name in ['peak_working_set', 'working_set',
        'peak_paged', 'paged', 'peak_nonpaged', 'nonpaged', 'pagefile', 'peak_pagefile', 'private']]


K.CreateToolhelp32Snapshot.argtypes = [W.DWORD, W.DWORD]
K.CreateToolhelp32Snapshot.restype = W.HANDLE
K.Process32FirstW.argtypes = [W.HANDLE, C.POINTER(ProcessEntry)]
K.Process32NextW.argtypes = [W.HANDLE, C.POINTER(ProcessEntry)]
K.OpenProcess.argtypes = [W.DWORD, W.BOOL, W.DWORD]
K.OpenProcess.restype = W.HANDLE
K.CloseHandle.argtypes = [W.HANDLE]
K.GetProcessTimes.argtypes = [W.HANDLE] + [C.POINTER(W.FILETIME)] * 4
P.GetProcessMemoryInfo.argtypes = [W.HANDLE, C.POINTER(Memory), W.DWORD]


def process_sample(root_pid):
    snapshot = K.CreateToolhelp32Snapshot(2, 0)
    entries = []
    try:
        entry = ProcessEntry()
        entry.size = C.sizeof(entry)
        available = K.Process32FirstW(snapshot, C.byref(entry))
        while available:
            entries.append((entry.pid, entry.parent, entry.name))
            available = K.Process32NextW(snapshot, C.byref(entry))
    finally:
        K.CloseHandle(snapshot)
    descendants = {root_pid}
    inference = {pid for pid, _, name in entries if name.lower().startswith('ollama')}
    for _ in range(len(entries)):
        expanded = descendants | {pid for pid, parent, _ in entries if parent in descendants}
        inference_expanded = inference | {pid for pid, parent, _ in entries if parent in inference}
        if expanded == descendants and inference_expanded == inference:
            break
        descendants = expanded
        inference = inference_expanded
    records = []
    for pid, parent, name in entries:
        group = 'prueba_cli' if pid in descendants else 'ollama' if pid in inference else None
        if group is None:
            continue
        record = {'pid': pid, 'parent_pid': parent, 'name': name, 'group': group}
        handle = K.OpenProcess(0x410, False, pid)
        if handle:
            try:
                memory = Memory()
                memory.size = C.sizeof(memory)
                if P.GetProcessMemoryInfo(handle, C.byref(memory), C.sizeof(memory)):
                    record.update(working_set_bytes=memory.working_set, private_bytes=memory.private,
                                  peak_working_set_bytes=memory.peak_working_set)
                times = [W.FILETIME() for _ in range(4)]
                if K.GetProcessTimes(handle, *(C.byref(value) for value in times)):
                    record['cpu_seconds'] = sum((value.dwHighDateTime << 32) + value.dwLowDateTime for value in times[2:]) / 10_000_000
            finally:
                K.CloseHandle(handle)
        else:
            record['error'] = C.get_last_error()
        records.append(record)
    return records


def snapshot():
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    result = {}
    for endpoint in ['version', 'tags', 'ps']:
        try:
            with opener.open('http://127.0.0.1:11434/api/' + endpoint, timeout=5) as response:
                result[endpoint] = json.load(response)
        except Exception as error:
            result[endpoint] = {'error': str(error)}
    return result


def gpu():
    command = ['nvidia-smi', '--query-gpu=name,memory.total,memory.used,memory.free,utilization.gpu', '--format=csv,noheader,nounits']
    result = subprocess.run(command, capture_output=True, text=True, timeout=10)
    return {'command': command, 'exit_code': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}


def run(label, command):
    evidence = ROOT / 'evidencias' / ('2026-10-20-e2-' + label)
    sources = [*ROOT.glob('src/**/*.rs'), ROOT / 'Cargo.toml', ROOT / 'Cargo.lock',
               ROOT / 'config/defaults.json', ROOT / 'tests/ollama_live.rs']
    record = {'date_utc': datetime.now(timezone.utc).isoformat(), 'command': command,
              'env': {key: os.environ.get(key) for key in ['PILL_LIVE_MODEL', 'PILL_LIVE_EVIDENCE']},
              'source_sha256': {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sources},
              'before': snapshot(), 'gpu_before': gpu(), 'samples': [],
              'measurement': 'Working set/private/CPU por proceso Windows. GPU global compartida. Muestreo aproximado 250 ms; GPU 1 s. No umbral aprobado.'}
    start = time.monotonic()
    with evidence.with_suffix('.stdout.txt').open('wb') as stdout, evidence.with_suffix('.stderr.txt').open('wb') as stderr:
        child_env = dict(os.environ, PYTHONIOENCODING='utf-8')
        process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr, env=child_env)
        last_gpu = -10
        while process.poll() is None:
            elapsed = time.monotonic() - start
            sample = {'seconds': elapsed, 'processes': process_sample(process.pid)}
            if elapsed - last_gpu >= 1:
                sample['gpu'] = gpu()
                last_gpu = elapsed
            record['samples'].append(sample)
            time.sleep(0.25)
        record['exit_code'] = process.returncode
    record.update(elapsed_seconds=time.monotonic() - start, after=snapshot(), gpu_after=gpu())
    record['test_binary_sha256'] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                                    for path in ROOT.glob('target/debug/deps/ollama_live-*.exe')}
    record['source_sha256_after'] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sources}
    record['stdout'] = evidence.with_suffix('.stdout.txt').read_text(encoding='utf-8', errors='replace')
    record['stderr'] = evidence.with_suffix('.stderr.txt').read_text(encoding='utf-8', errors='replace')
    if (ROOT / 'bin/pill.exe').exists():
        record['bin_sha256'] = hashlib.sha256((ROOT / 'bin/pill.exe').read_bytes()).hexdigest()
    evidence.with_suffix('.json').write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({'evidence': str(evidence.with_suffix('.json')), 'exit_code': record['exit_code'],
                      'elapsed_seconds': record['elapsed_seconds'], 'stdout': record['stdout'], 'stderr': record['stderr']}, ensure_ascii=True))
    return record['exit_code']


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('label')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    raise SystemExit(run(args.label, args.command))
