"""E3/E4: consola Windows, permisos, agentes y herramientas locales reales.

Solo usa un proyecto y concesiones temporales; modelo y límites por argumento.
El informe conserva resultados, diarios y capturas también cuando hay fallos.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import test_console as console
import measure_e2 as measurements


def command(text):
    console.press(text)
    console.press(vk=0x0D)


def submit(text):
    console.press(text)
    console.press(vk=0x0D, control=0x0008)


def run(args):
    binary = Path(args.binary).resolve()
    report = {'fecha_utc': datetime.now(timezone.utc).isoformat(),
              'binario': str(binary), 'sha256_binario': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'modelo_explicito': args.model, 'num_ctx': args.num_ctx, 'num_predict': args.num_predict,
              'think': args.think,
              'alcance': ('Consola Win32, metadatos Ollama y permisos reales; sin inferencia ni ejecución por el producto.' if args.permissions_only else 'Consola Win32, inferencia Ollama, agentes y herramientas reales; proyecto y concesiones descartables; sin descargas.'),
              'checks': {}, 'capturas': {}, 'operaciones': {}, 'errores': [],
              'recursos': {'metodo': 'Win32 Toolhelp/GetProcessMemoryInfo/GetProcessTimes por PID. Host, CLI, pruebas y Ollama separados por registro. CPU acumulada; deltas solo entre muestras observadas, sin umbral ni promesa de consumo máximo. Muestreo durante inferencia aproximadamente cada 250 ms.', 'reposo': []}}
    captures, checks = report['capturas'], report['checks']
    host = None
    attached = False
    with tempfile.TemporaryDirectory(prefix='pill-e34-console-') as directory:
        folder = Path(directory)
        project = folder / 'Proyecto ñ'
        (project / '.pillcli').mkdir(parents=True)
        private = folder / 'private'
        private.mkdir()
        trust = private / 'PILLCLI/trust.json'
        # Igual que el fixture Rust: bytes LF explícitos, sin traducción Windows.
        original = b'def suma(a, b):\n    return a - b\n'
        (project / 'calc.py').write_bytes(original)
        report['fixture_bytes_originales_hex'] = original.hex()
        test_source = "import calc\nassert calc.suma(2,3)==5, 'suma incorrecta'\nassert calc.suma(-2,3)==1\nprint('PRUEBA_SUMA_PASS')\n"
        (project / 'test_calc.py').write_text(test_source, encoding='utf-8')
        fixture_command = [args.python, '-B', 'test_calc.py']
        before_test = subprocess.run(fixture_command, cwd=project, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        report['prueba_antes'] = {'codigo': before_test.returncode, 'stdout': before_test.stdout, 'stderr': before_test.stderr}
        checks['fixture_falla_antes'] = before_test.returncode != 0
        config = {'ui': {'poll_ms': 40, 'max_history_entries': 256},
                  'ollama': {'model': args.model, 'num_ctx': args.num_ctx, 'num_predict': args.num_predict, 'think': args.think},
                  'commands': {'commands': [{'id': 'test_suma', 'program': args.python,
                                             'args': ['-B', 'test_calc.py'], 'cwd': '.', 'watched_files': ['test_calc.py']}]}}
        (project / '.pillcli/config.json').write_text(json.dumps(config), encoding='utf-8')
        startup = subprocess.STARTUPINFO()
        startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
        child_env = os.environ.copy()
        child_env['LOCALAPPDATA'] = str(private)
        try:
            host = subprocess.Popen([shutil.which('pwsh'), '-NoProfile', '-File', str(console.ROOT / 'diagnostics/console_host.ps1'),
                                     directory, str(binary), str(project)], env=child_env,
                                    creationflags=subprocess.CREATE_NEW_CONSOLE, startupinfo=startup)
            console.wait_for(lambda: (folder / 'ready').exists(), 'host')
            console.K.FreeConsole()
            console.check(console.K.AttachConsole(host.pid))
            attached = True
            report['estado_antes'] = console.state()
            (folder / 'start').touch()
            console.wait_for(lambda: 'Nueva conversación' in console.screen(), 'modelo explícito seleccionado', seconds=30)
            captures['inicio'] = console.screen()
            checks['modelo_explicito'] = args.model in captures['inicio'] and 'Digest:' in captures['inicio']
            report['recursos']['reposo'].append({'fecha_utc': datetime.now(timezone.utc).isoformat(), 'procesos': measurements.process_sample(host.pid)})
            time.sleep(0.3)
            report['recursos']['reposo'].append({'fecha_utc': datetime.now(timezone.utc).isoformat(), 'procesos': measurements.process_sample(host.pid)})
            command('/permissions')
            console.wait_for(lambda: 'consentimiento explícito' in console.screen(), 'propuestas')
            captures['propuesta'] = console.screen()
            checks['propuesta_real'] = 'test_suma' in captures['propuesta']
            # Enter representativo de un salto pegado: jamás debe conceder.
            console.press('1')
            console.press(vk=0x0D)
            console.press(vk=0x0D)
            time.sleep(0.3)
            captures['enter_sin_concesion'] = console.screen()
            checks['enter_no_concede'] = not trust.exists() and 'Comando concedido' not in console.screen()
            # Conserva la misma propuesta vigente; retira los dos saltos antes
            # del consentimiento explícito. No hay una segunda consulta en vuelo.
            for _ in range(2):
                console.press(vk=0x08)
            submit('')
            console.wait_for(lambda: 'Comando concedido' in console.screen(), 'concesión explícita')
            captures['concesion'] = console.screen()
            checks['ctrl_enter_concede'] = trust.exists()
            report['concesion'] = json.loads(trust.read_text(encoding='utf-8'))
            command('/task')
            console.wait_for(lambda: 'Modo tarea:' in console.screen(), 'modo tarea')
            captures['modo_tarea'] = console.screen()
            checks['modo_tarea'] = True
            if not args.permissions_only:
                prompt = ('Corrige calc.py: suma debe sumar a y b. Lee calc.py con read_file, usa edit_file con su expected_sha256 real '
                          'y cambia únicamente a - b por a + b. No edites test_calc.py. Ejecuta run_command con id test_suma '
                          'y comprueba resultado. Reviewer debe leer calc.py antes de APPROVE.')
                submit(prompt)
                report['operaciones']['tarea'] = observe_operation(args.timeout, captures, 'tarea', host.pid)
                checks['tarea_completa'] = report['operaciones']['tarea']['estado'] == 'complete'
            command('/chat')
            console.wait_for(lambda: 'Modo chat con herramientas' in console.screen(), 'modo chat')
            captures['modo_chat'] = console.screen()
            checks['modo_chat'] = True
            if not args.permissions_only:
                submit('Lee calc.py mediante read_file y dime qué operación realiza suma. No edites archivos ni ejecutes comandos.')
                report['operaciones']['chat'] = observe_operation(args.timeout, captures, 'chat', host.pid)
                checks['chat_completo'] = report['operaciones']['chat']['estado'] == 'complete'
            command('/exit')
            console.wait_for(lambda: (folder / 'result').exists(), 'salida')
            report['codigo_salida'] = int((folder / 'result').read_text(encoding='utf-8-sig').strip())
            report['estado_despues'] = console.state()
            checks['salida_correcta'] = report['codigo_salida'] == 0
            checks['restauracion'] = report['estado_antes'] == report['estado_despues']
        except Exception as error:
            report['errores'].append(repr(error))
            if attached:
                captures['fallo'] = console.screen()
        finally:
            if host is not None:
                if host.poll() is None and not (folder / 'result').exists() and attached:
                    console.press('\x03')
                    try:
                        console.wait_for(lambda: (folder / 'result').exists(), 'salida tras fallo', seconds=15)
                        report['estado_despues'] = console.state()
                        checks['restauracion_tras_fallo'] = report.get('estado_antes') == report['estado_despues']
                    except Exception as error:
                        report['errores'].append(repr(error))
                (folder / 'finish').touch()
                try:
                    host.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    host.terminate()
                    host.wait(timeout=5)
            if attached:
                console.K.FreeConsole()
            report['contenido_final'] = (project / 'calc.py').read_text(encoding='utf-8')
            report['fixture_bytes_finales_hex'] = (project / 'calc.py').read_bytes().hex()
            report['diarios'] = []
            sessions = project / '.pillcli/sessions'
            if sessions.exists():
                for path in sorted(sessions.iterdir()):
                    records = [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()]
                    report['diarios'].append({'nombre': path.name, 'registros': records})
            if not args.permissions_only:
                after_test = subprocess.run(fixture_command, cwd=project, stdin=subprocess.DEVNULL, capture_output=True, text=True)
                report['prueba_despues'] = {'codigo': after_test.returncode, 'stdout': after_test.stdout, 'stderr': after_test.stderr}
                checks['fixture_pasa_despues'] = after_test.returncode == 0 and 'PRUEBA_SUMA_PASS' in after_test.stdout
                checks['pruebas_sin_modificar'] = (project / 'test_calc.py').read_text(encoding='utf-8') == test_source
                records = [record for journal in report['diarios'] for record in journal['registros']]
                report['registros_por_tipo'] = sorted({str(record.get('stage')) for record in records})
                checks['diarios_reales'] = bool(records)
                checks['sesiones_tarea_chat_separadas'] = len(report['diarios']) == 2
                if report['diarios']:
                    checks['chat_lectura_real'] = any(record.get('stage') == 'observed' and record.get('tool') == 'read_file' and record.get('status') == 'verified' for record in report['diarios'][-1]['registros'])
                report['roles_observados'] = sorted({str(record.get('agent')) for record in records})
                checks['cinco_roles_reales'] = all(any(record.get('agent') == role and record.get('stage') == 'agent_started' for record in records) for role in ('coordinator', 'explorer', 'implementer', 'tester', 'reviewer'))
                observed = [record for record in records if record.get('stage') == 'observed']
                for name in ('read_file', 'edit_file', 'run_command'):
                    checks['herramienta_real_' + name] = any(record.get('tool') == name and record.get('status') == 'verified' for record in observed)
                checks['comando_exitoso_en_diario'] = any(record.get('tool') == 'run_command' and record.get('command', {}).get('exit_code') == 0 for record in observed)
                checks['tester_comando_real'] = any(record.get('agent') == 'tester' and record.get('tool') == 'run_command' and record.get('status') == 'verified' for record in observed)
                checks['reviewer_lectura_real'] = any(record.get('agent') == 'reviewer' and record.get('tool') == 'read_file' and record.get('path') == 'calc.py' and record.get('status') == 'verified' for record in observed)
                requests = {record['call_id'] for record in records if record.get('stage') == 'requested'}
                checks['resultados_vinculados'] = bool(observed) and all(record['call_id'] in requests for record in observed)
                screens = [screen for operation in report['operaciones'].values() for screen in operation['pantallas']]
                checks['resultados_visibles'] = any('read_file:' in screen or 'edit_file:' in screen or 'run_command:' in screen for screen in screens)
        samples = list(report['recursos']['reposo'])
        for operation in report['operaciones'].values():
            samples.extend(operation['recursos'])
        per_pid = {}
        for sample in samples:
            for process in sample['procesos']:
                per_pid.setdefault(process['pid'], []).append(process)
        report['recursos']['resumen_por_pid'] = []
        for pid, processes in per_pid.items():
            cpu = [process['cpu_seconds'] for process in processes if 'cpu_seconds' in process]
            summary = {'pid': pid, 'nombre': processes[0]['name'], 'grupo': processes[0]['group'], 'muestras': len(processes)}
            for field in ('working_set_bytes', 'private_bytes', 'peak_working_set_bytes'):
                values = [process[field] for process in processes if field in process]
                if values:
                    summary[field + '_max_observado'] = max(values)
            if cpu:
                summary['cpu_delta_seconds_observado'] = max(cpu) - min(cpu)
            report['recursos']['resumen_por_pid'].append(summary)
        checks['recursos_cli_memoria_cpu'] = any(summary['nombre'].lower() == 'pill.exe'
                                               and 'working_set_bytes_max_observado' in summary
                                               and 'cpu_delta_seconds_observado' in summary
                                               for summary in report['recursos']['resumen_por_pid'])
        report['resultado'] = 'PASS' if checks and all(checks.values()) and not report['errores'] else 'FAIL'
    return report


def observe_operation(timeout, captures, label, host_pid):
    start = time.monotonic()
    observed = []
    previous = None
    activated = False
    resources = []
    last_resource = -1
    while time.monotonic() - start < timeout:
        screen = console.screen()
        if time.monotonic() - last_resource >= 0.25:
            resources.append({'fecha_utc': datetime.now(timezone.utc).isoformat(), 'segundos': time.monotonic() - start,
                              'procesos': measurements.process_sample(host_pid)})
            last_resource = time.monotonic()
        if screen != previous:
            observed.append(screen)
            previous = screen
        if 'Generando ·' in screen or 'Trabajando ·' in screen:
            activated = True
        if activated and 'Respuesta completa' in screen:
            captures[label] = screen
            return {'estado': 'complete', 'duracion_s': time.monotonic() - start, 'pantallas': observed, 'recursos': resources}
        if activated and ('Operación fallida' in screen or 'Cancelado ·' in screen):
            captures[label] = screen
            return {'estado': 'failed', 'duracion_s': time.monotonic() - start, 'pantallas': observed, 'recursos': resources}
        time.sleep(0.1)
    console.press(vk=0x1B)
    console.wait_for(lambda: 'Cancelado ·' in console.screen() or 'Operación fallida' in console.screen(), 'cancelación tras timeout', seconds=30)
    captures[label] = console.screen()
    return {'estado': 'timeout', 'duracion_s': time.monotonic() - start, 'pantallas': observed, 'recursos': resources}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--model', required=True)
    parser.add_argument('--num-ctx', type=int, required=True)
    parser.add_argument('--num-predict', type=int, required=True)
    parser.add_argument('--think', action=argparse.BooleanOptionalAction, default=False)
    parser.add_argument('--python', required=True)
    parser.add_argument('--binary', default=str(console.ROOT / 'bin/pill.exe'))
    parser.add_argument('--timeout', type=int, default=600)
    parser.add_argument('--permissions-only', action='store_true')
    parser.add_argument('--evidence', required=True)
    args = parser.parse_args()
    report = run(args)
    Path(args.evidence).write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({'resultado': report['resultado'], 'checks': report['checks'], 'errores': report['errores']}, ensure_ascii=True))
    raise SystemExit(0 if report['resultado'] == 'PASS' else 1)
