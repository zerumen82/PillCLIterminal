"""R13 real: servidor Ollama propio, consentimiento temporal y recursos separados.

Solo biblioteca estándar. No instala, descarga modelos ni llama a /api/delete.
Un Job Object propio conserva la identidad de los procesos que puede terminar.
"""
import argparse
import ctypes as C
from ctypes import wintypes as W
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
import urllib.parse
import urllib.request

import measure_e2 as measurements


ROOT = Path(__file__).resolve().parents[1]
K = C.WinDLL('kernel32', use_last_error=True)
I = C.WinDLL('iphlpapi', use_last_error=True)
K.OpenProcess.argtypes = [W.DWORD, W.BOOL, W.DWORD]
K.OpenProcess.restype = W.HANDLE
K.CloseHandle.argtypes = [W.HANDLE]
K.QueryFullProcessImageNameW.argtypes = [W.HANDLE, W.DWORD, W.LPWSTR, C.POINTER(W.DWORD)]
K.QueryFullProcessImageNameW.restype = W.BOOL
K.GetProcessTimes.argtypes = [W.HANDLE] + [C.POINTER(W.FILETIME)] * 4
K.GetProcessTimes.restype = W.BOOL
I.GetExtendedTcpTable.argtypes = [C.c_void_p, C.POINTER(W.DWORD), W.BOOL, W.ULONG, C.c_int, W.ULONG]
I.GetExtendedTcpTable.restype = W.DWORD


class TcpRow(C.Structure):
    _fields_ = [(name, W.DWORD) for name in ['state', 'local_address', 'local_port',
                                          'remote_address', 'remote_port', 'pid']]


class BasicLimits(C.Structure):
    _fields_ = [('process_time', C.c_longlong), ('job_time', C.c_longlong),
                ('flags', W.DWORD), ('min_working_set', C.c_size_t),
                ('max_working_set', C.c_size_t), ('active_processes', W.DWORD),
                ('affinity', C.c_size_t), ('priority', W.DWORD), ('scheduling', W.DWORD)]


class IoCounters(C.Structure):
    _fields_ = [(name, C.c_ulonglong) for name in ['read_ops', 'write_ops', 'other_ops',
                                                'read_bytes', 'write_bytes', 'other_bytes']]


class ExtendedLimits(C.Structure):
    _fields_ = [('basic', BasicLimits), ('io', IoCounters),
                ('process_memory', C.c_size_t), ('job_memory', C.c_size_t),
                ('peak_process_memory', C.c_size_t), ('peak_job_memory', C.c_size_t)]


K.CreateJobObjectW.argtypes = [C.c_void_p, W.LPCWSTR]
K.CreateJobObjectW.restype = W.HANDLE
K.SetInformationJobObject.argtypes = [W.HANDLE, C.c_int, C.c_void_p, W.DWORD]
K.SetInformationJobObject.restype = W.BOOL
K.AssignProcessToJobObject.argtypes = [W.HANDLE, W.HANDLE]
K.AssignProcessToJobObject.restype = W.BOOL


class OwnedJob:
    def __init__(self):
        self.handle = K.CreateJobObjectW(None, None)
        if not self.handle:
            raise C.WinError(C.get_last_error())
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not K.SetInformationJobObject(self.handle, 9, C.byref(limits), C.sizeof(limits)):
            self.close()
            raise C.WinError(C.get_last_error())

    def assign(self, process):
        if not K.AssignProcessToJobObject(self.handle, W.HANDLE(int(process._handle))):
            # Se termina solo el Popen recién creado, nunca una búsqueda por nombre.
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            raise C.WinError(C.get_last_error())

    def close(self):
        if self.handle:
            K.CloseHandle(self.handle)
            self.handle = None


def utc():
    return datetime.now(timezone.utc).isoformat()


def json_hash(value):
    encoded = json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(',', ':')).encode('utf-8')
    return hashlib.sha256(encoded).hexdigest()


def owners(port):
    size = W.DWORD()
    error = I.GetExtendedTcpTable(None, C.byref(size), False, socket.AF_INET, 5, 0)
    if error not in (0, 122):
        raise C.WinError(error)
    for _ in range(4):
        buffer = C.create_string_buffer(size.value)
        error = I.GetExtendedTcpTable(buffer, C.byref(size), False, socket.AF_INET, 5, 0)
        if error == 122:
            continue
        if error:
            raise C.WinError(error)
        count = W.DWORD.from_buffer_copy(buffer.raw[:C.sizeof(W.DWORD)]).value
        rows = [TcpRow.from_buffer_copy(buffer.raw[C.sizeof(W.DWORD) + index * C.sizeof(TcpRow):
                                                  C.sizeof(W.DWORD) + (index + 1) * C.sizeof(TcpRow)])
                for index in range(count)]
        return sorted({row.pid for row in rows
                       if row.state == 2 and socket.ntohs(row.local_port & 0xffff) == port})
    raise RuntimeError('la tabla TCP cambia demasiado para acreditar el propietario')


def identity(pid):
    record = {'pid': pid}
    handle = K.OpenProcess(0x1000, False, pid)
    if not handle:
        return dict(record, error=C.get_last_error())
    try:
        path = C.create_unicode_buffer(32768)
        size = W.DWORD(len(path))
        if K.QueryFullProcessImageNameW(handle, 0, path, C.byref(size)):
            record['executable'] = path.value
            try:
                record['executable_sha256'] = hashlib.sha256(Path(path.value).read_bytes()).hexdigest()
            except OSError as error:
                record['hash_error'] = str(error)
        times = [W.FILETIME() for _ in range(4)]
        if K.GetProcessTimes(handle, *(C.byref(value) for value in times)):
            record['creation_ticks'] = (times[0].dwHighDateTime << 32) + times[0].dwLowDateTime
    finally:
        K.CloseHandle(handle)
    return record


def python_processes():
    snapshot = measurements.K.CreateToolhelp32Snapshot(2, 0)
    found = []
    try:
        entry = measurements.ProcessEntry()
        entry.size = C.sizeof(entry)
        available = measurements.K.Process32FirstW(snapshot, C.byref(entry))
        while available:
            if entry.pid != os.getpid() and entry.name.lower().startswith(('python', 'comfyui')):
                found.append(identity(entry.pid))
            available = measurements.K.Process32NextW(snapshot, C.byref(entry))
    finally:
        measurements.K.CloseHandle(snapshot)
    return found


def api(url, endpoint, limit, timeout=3):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(url.rstrip('/') + '/api/' + endpoint, timeout=timeout) as response:
        body = response.read(limit + 1)
    if len(body) > limit:
        raise RuntimeError('respuesta de metadatos supera el límite configurado')
    return json.loads(body)


def api_snapshot(url, limit):
    record = {'fecha_utc': utc(), 'url': url}
    for endpoint in ['version', 'tags', 'ps']:
        try:
            value = api(url, endpoint, limit)
            record[endpoint] = {'sha256': json_hash(value), 'value': value}
        except Exception as error:
            record[endpoint] = {'error': str(error)}
    return record


def gpu_snapshot(program):
    if program is None:
        return {'available': False, 'reason': 'nvidia-smi no encontrado; no se inventa VRAM'}
    command = [str(program), '--query-gpu=name,memory.total,memory.used,memory.free,utilization.gpu',
               '--format=csv,noheader,nounits']
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=10,
                                creationflags=subprocess.CREATE_NO_WINDOW)
        return {'fecha_utc': utc(), 'available': result.returncode == 0, 'command': command,
                'exit_code': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr,
                'scope': 'VRAM global compartida; delta no atribuible exclusivamente al diagnóstico'}
    except Exception as error:
        return {'available': False, 'error': str(error), 'command': command}


def same_process(before, after):
    keys = ['pid', 'creation_ticks', 'executable_sha256']
    return all(before.get(key) is not None and after.get(key) is not None
               and before[key] == after[key] for key in keys)


def run(args):
    defaults = json.loads((ROOT / 'config/defaults.json').read_text(encoding='utf-8'))
    shared_url = args.shared_url or defaults['ollama']['url']
    parsed = urllib.parse.urlsplit(shared_url)
    if parsed.scheme != 'http' or parsed.hostname not in ('localhost', '127.0.0.1'):
        raise ValueError('el servidor compartido de control debe usar HTTP loopback IPv4')
    shared_port = parsed.port or 80
    evidence = Path(args.evidence).resolve()
    if evidence.exists():
        raise FileExistsError(f'No se sobrescribe una evidencia existente: {evidence}')
    evidence.parent.mkdir(parents=True, exist_ok=True)
    artifacts = {name: evidence.with_suffix(suffix) for name, suffix in {
        'rust': '.rust.json', 'server_stdout': '.server.stdout.log', 'server_stderr': '.server.stderr.log',
        'cargo_stdout': '.cargo.stdout.log', 'cargo_stderr': '.cargo.stderr.log'}.items()}
    if any(path.exists() for path in artifacts.values()):
        raise FileExistsError('ya existe un artefacto de esta ejecución; elige otra evidencia')
    ollama = Path(args.ollama or shutil.which('ollama') or '')
    if not ollama.is_file():
        raise FileNotFoundError('Ollama no está instalado; no se instala ni descarga automáticamente')
    cargo = shutil.which('cargo')
    if cargo is None:
        raise FileNotFoundError('cargo no disponible')
    nvidia = shutil.which('nvidia-smi')
    report = {'fecha_utc': utc(), 'result': 'FAIL', 'model': args.model, 'artifacts': {k: str(v) for k, v in artifacts.items()},
              'samples': [], 'checks': {}, 'scope': 'Servidor propio, puerto distinto, Job Object propio; servidor compartido consultado solo por GET. No instalaciones, modelos nuevos, /api/delete ni terminación de procesos ajenos.'}
    source_paths = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'config/defaults.json',
                    ROOT / 'tests/gpu_live.rs', Path(__file__).resolve(), *sorted((ROOT / 'src').rglob('*.rs'))]
    report['source_sha256'] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in source_paths}
    server = runner = job = None
    handles = []
    started = time.monotonic()
    metadata_limit = defaults['ollama']['max_metadata_bytes']
    timeout = args.timeout or defaults['ollama']['response_timeout_ms'] / 1000 * 3
    try:
        report['shared_before'] = api_snapshot(shared_url, metadata_limit)
        if any('error' in report['shared_before'][endpoint] for endpoint in ['version', 'tags', 'ps']):
            raise RuntimeError('no se puede acreditar el servidor compartido antes de la prueba')
        report['shared_processes_before'] = [identity(pid) for pid in owners(shared_port)]
        report['foreign_python_before'] = python_processes()
        report['gpu_before'] = gpu_snapshot(nvidia)
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as reservation:
            reservation.bind(('127.0.0.1', 0))
            port = reservation.getsockname()[1]
        if port in (shared_port, 11434) or owners(port):
            raise RuntimeError('el puerto reservado coincide con un listener ajeno o habitual')
        own_url = f'http://127.0.0.1:{port}'
        child_env = dict(os.environ, OLLAMA_HOST=own_url)
        server_stdout = artifacts['server_stdout'].open('wb')
        server_stderr = artifacts['server_stderr'].open('wb')
        handles += [server_stdout, server_stderr]
        job = OwnedJob()
        server = subprocess.Popen([str(ollama.resolve()), 'serve'], cwd=ROOT, env=child_env,
                                  stdin=subprocess.DEVNULL, stdout=server_stdout, stderr=server_stderr,
                                  creationflags=subprocess.CREATE_NO_WINDOW)
        job.assign(server)
        report['own_server'] = {'url': own_url, 'pid': server.pid, 'identity': identity(server.pid),
                                'model_store': os.environ.get('OLLAMA_MODELS', 'almacén predeterminado instalado del usuario')}
        while True:
            if server.poll() is not None:
                raise RuntimeError(f'el servidor propio terminó con código {server.returncode}')
            listener_pids = owners(port)
            if listener_pids and listener_pids != [server.pid]:
                raise RuntimeError(f'listener no pertenece al proceso propio: {listener_pids}')
            if listener_pids == [server.pid]:
                try:
                    report['own_server']['version'] = api(own_url, 'version', metadata_limit)
                    break
                except Exception:
                    pass
            if time.monotonic() - started > timeout:
                raise TimeoutError('servidor propio no preparado')
            time.sleep(0.1)
        own_tags = api(own_url, 'tags', metadata_limit)
        if not any(model['name'] == args.model for model in own_tags.get('models', [])):
            raise RuntimeError('modelo explícito no instalado en el almacén; no se descarga')
        report['checks']['own_listener_pid_verified'] = True
        command = [cargo, 'test', '--locked', '--offline', '--test', 'gpu_live',
                   'real_exclusive_release_preserves_shared_mode_and_unloads_owned_model',
                   '--', '--ignored', '--exact', '--nocapture']
        report['command'] = command
        runner_env = dict(os.environ, PILL_RELEASE_URL=own_url, PILL_LIVE_MODEL=args.model,
                          PILL_LIVE_EVIDENCE=str(artifacts['rust']), PILL_LIVE_CONTEXT=str(args.num_ctx),
                          PILL_LIVE_PREDICT=str(args.num_predict), PILL_LIVE_THINK=str(args.think).lower())
        cargo_stdout = artifacts['cargo_stdout'].open('wb')
        cargo_stderr = artifacts['cargo_stderr'].open('wb')
        handles += [cargo_stdout, cargo_stderr]
        runner = subprocess.Popen(command, cwd=ROOT, env=runner_env, stdin=subprocess.DEVNULL,
                                  stdout=cargo_stdout, stderr=cargo_stderr,
                                  creationflags=subprocess.CREATE_NO_WINDOW)
        job.assign(runner)
        while runner.poll() is None:
            if time.monotonic() - started > timeout:
                raise TimeoutError('la prueba real agotó el tiempo; solo se detendrá el Job propio')
            report['samples'].append({'seconds': time.monotonic() - started,
                                      'gpu': gpu_snapshot(nvidia),
                                      'processes': measurements.process_sample(runner.pid)})
            time.sleep(1)
        report['cargo_exit_code'] = runner.returncode
        if artifacts['rust'].exists():
            report['rust'] = json.loads(artifacts['rust'].read_text(encoding='utf-8'))
        report['checks']['cargo_pass'] = runner.returncode == 0 and report.get('rust', {}).get('result') == 'PASS'
        report['checks']['own_model_used_gpu'] = report.get('rust', {}).get('model_was_on_gpu') is True
        report['checks']['own_listener_still_owned'] = owners(port) == [server.pid]
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        report['gpu_after_test'] = gpu_snapshot(nvidia)
        report['shared_after'] = api_snapshot(shared_url, metadata_limit)
        try:
            report['shared_processes_after'] = [identity(pid) for pid in owners(shared_port)]
            report['checks']['shared_process_identity_preserved'] = bool(report.get('shared_processes_before')) and all(
                any(same_process(before, after) for after in report['shared_processes_after'])
                for before in report.get('shared_processes_before', []))
            report['foreign_python_after'] = [identity(before['pid']) for before in report.get('foreign_python_before', [])]
            report['checks']['foreign_python_identity_preserved'] = all(
                same_process(before, after) for before, after in zip(report.get('foreign_python_before', []), report['foreign_python_after']))
            report['checks']['shared_installed_models_unchanged'] = (
                report.get('shared_before', {}).get('tags', {}).get('sha256') is not None
                and report['shared_before']['tags']['sha256'] == report['shared_after']['tags'].get('sha256'))
        except Exception as error:
            report['after_error'] = str(error)
        if job is not None:
            job.close()  # Mata únicamente los procesos que este harness asignó explícitamente.
        for process in [runner, server]:
            if process is not None:
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    # El handle Popen sigue identificando exclusivamente nuestro proceso.
                    process.terminate()
                    process.wait(timeout=5)
        for handle in handles:
            handle.close()
        report['own_processes_stopped'] = all(process is None or process.poll() is not None for process in [runner, server])
        report['gpu_after_cleanup'] = gpu_snapshot(nvidia)
        report['elapsed_seconds'] = time.monotonic() - started
        report['result'] = 'PASS' if (not report.get('error') and report['checks']
                                      and all(report['checks'].values()) and report['own_processes_stopped']) else 'FAIL'
        evidence.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(f"{report['result']}: servidor propio, conservación compartida y descarga exclusiva; evidencia {evidence}")
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--model', required=True)
    parser.add_argument('--evidence', required=True)
    parser.add_argument('--ollama')
    parser.add_argument('--shared-url')
    parser.add_argument('--num-ctx', type=int, required=True)
    parser.add_argument('--num-predict', type=int, required=True)
    parser.add_argument('--think', action=argparse.BooleanOptionalAction, default=False)
    parser.add_argument('--timeout', type=float)
    raise SystemExit(run(parser.parse_args()))
