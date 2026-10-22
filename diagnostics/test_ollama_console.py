"""Consola real + Ollama real; modelo por argumento, sin descargas/liberación."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import test_console as console
import measure_e2 as measurements


def resource_window(host_pid):
    started = time.monotonic()
    before = measurements.process_sample(host_pid)
    time.sleep(0.5)
    after = measurements.process_sample(host_pid)
    return {'interval_seconds': time.monotonic() - started, 'processes_before': before,
            'processes_after': after, 'gpu': measurements.gpu(),
            'scope': 'Procesos CLI/host y Ollama separados; GPU global compartida; CPU acumulada en segundos por PID.'}


def run(model):
    captures = {}
    resources = {}
    with tempfile.TemporaryDirectory(prefix='pill-e2-') as directory:
        folder = Path(directory)
        project = folder / 'Proyecto ñ'
        (project / '.pillcli').mkdir(parents=True)
        (project / '.pillcli/config.json').write_text(json.dumps({'ui': {'poll_ms': 40}}), encoding='utf-8')
        startup = subprocess.STARTUPINFO()
        startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
        host = subprocess.Popen([shutil.which('pwsh'), '-NoProfile', '-File', str(console.ROOT / 'diagnostics/console_host.ps1'), directory,
                                 str(console.ROOT / 'bin/pill.exe'), str(project)], creationflags=subprocess.CREATE_NEW_CONSOLE, startupinfo=startup)
        try:
            console.wait_for(lambda: (folder / 'ready').exists(), 'host')
            console.K.FreeConsole()
            console.check(console.K.AttachConsole(host.pid))
            before = console.state()
            (folder / 'start').touch()
            console.wait_for(lambda: 'Elige un número' in console.screen(), 'modelos reales', seconds=15)
            captures['seleccion_inicial'] = console.screen()
            choices = [line.strip() for line in captures['seleccion_inicial'].splitlines() if model in line]
            assert choices, (model, captures['seleccion_inicial'])
            index = choices[0].split(' · ')[0]
            console.press(index)
            console.press(vk=0x0D, control=0x0008)
            console.wait_for(lambda: 'Nueva conversación' in console.screen(), 'modelo seleccionado', seconds=15)
            captures['modelo_y_digest'] = console.screen()
            resources['reposo_modelo_seleccionado'] = resource_window(host.pid)
            console.press('Responde solamente con la palabra')
            console.press(vk=0x0D)
            console.press('CAPSULA')
            time.sleep(0.15)
            assert 'Generando' not in console.screen(), 'Enter no debe enviar'
            captures['pegado_sin_envio'] = console.screen()
            console.press(vk=0x0D, control=0x0008)
            console.wait_for(lambda: 'Respuesta completa' in console.screen(), 'respuesta Ollama', seconds=200)
            captures['respuesta_real'] = console.screen()
            assert 'CAPSULA' in captures['respuesta_real']
            # La petición de una sola palabra no debe sesgar la prueba de generación larga.
            console.press('/model')
            console.press(vk=0x0D)
            console.wait_for(lambda: 'Selección explícita de modelo' in console.screen(), 'nueva selección explícita')
            console.press(index)
            console.press(vk=0x0D, control=0x0008)
            console.wait_for(lambda: 'Preparado · herramientas pendientes' in console.screen(), 'nueva conversación preparada')
            captures['nueva_conversacion'] = console.screen()
            console.press('Enumera del uno al mil con explicaciones detalladas.')
            console.press(vk=0x0D, control=0x0008)
            console.wait_for(lambda: 'Generando' in console.screen(), 'generación activa')
            console.press('áXYZz')
            console.wait_for(lambda: '> áXYZz' in console.screen() and 'Generando' in console.screen(), 'entrada durante inferencia')
            captures['entrada_durante_inferencia'] = console.screen()
            resources['generacion_activa'] = {'processes': measurements.process_sample(host.pid)}
            old_size = console.dimensions()
            console.check(console.U.SetWindowPos(console.K.GetConsoleWindow(), None, 0, 0, 720, 440, 0x0016))
            console.wait_for(lambda: console.dimensions()[0] < old_size[0] and '> áXYZz' in console.screen() and 'Generando' in console.screen(), 'redimensionado durante inferencia')
            captures['entrada_resize_durante_inferencia'] = console.screen()
            console.press(vk=0x1B)
            console.wait_for(lambda: 'Operación cancelada' in console.screen(), 'cancelación real')
            captures['cancelacion'] = console.screen()
            console.press('\x03')
            console.wait_for(lambda: (folder / 'result').exists(), 'restauración y salida')
            assert int((folder / 'result').read_text(encoding='utf-8-sig').strip()) == 0
            after = console.state()
            assert before == after
            return {'resultado': 'PASS', 'estado_antes': before, 'estado_despues': after, 'capturas': captures,
                    'recursos': resources,
                    'resize': {'antes': old_size, 'despues': console.dimensions()}}
        except Exception as error:
            failure = {'error': repr(error), 'captures': captures, 'resources': resources,
                       'screen': console.screen(), 'state': console.state(), 'dimensions': console.dimensions()}
            (console.ROOT / 'evidencias/2026-10-20-e2-consola-fallo.json').write_text(
                json.dumps(failure, ensure_ascii=True, indent=2), encoding='utf-8')
            raise
        finally:
            (folder / 'finish').touch()
            if host.poll() is None and not (folder / 'result').exists():
                console.press('\x03')
            try:
                host.wait(timeout=5)
            except subprocess.TimeoutExpired:
                host.terminate()
                host.wait(timeout=3)
            console.K.FreeConsole()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--model', required=True)
    args = parser.parse_args()
    report = run(args.model)
    report.update({'fecha_utc': datetime.now(timezone.utc).isoformat(), 'modelo_elegido_prueba': args.model,
                   'sha256_binario': hashlib.sha256((console.ROOT / 'bin/pill.exe').read_bytes()).hexdigest(),
                   'alcance': 'Consola Windows y Ollama reales; no herramientas, agentes ni liberación'})
    (console.ROOT / 'evidencias/2026-10-20-e2-consola.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print('PASS: elección, digest, pegado sin envío, Ctrl+Enter, chat, entrada/resize durante inferencia, Esc y restauración.')
