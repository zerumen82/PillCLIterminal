"""Consola Win32 real: edición, permisos, scroll, persistencia y recuperación.

No genera respuestas de modelo. El único fallo inyectado es un diario truncado
en una muestra descartable; se conserva y se identifica en la evidencia.
"""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import test_console as console


def command(text):
    console.press(text)
    console.press(vk=0x0D)


def draft(text):
    return ('> ' + text) in console.screen().splitlines()


def clear_line(length):
    console.press(vk=0x24)
    for _ in range(length):
        console.press(vk=0x2E)
    console.wait_for(lambda: '>' in console.screen().splitlines(), 'borrador vacío')


@contextmanager
def session(folder, project, binary, private, report):
    folder.mkdir()
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    environment = os.environ.copy()
    environment['LOCALAPPDATA'] = str(private)
    host = subprocess.Popen(
        [shutil.which('pwsh'), '-NoProfile', '-File',
         str(console.ROOT / 'diagnostics/console_host.ps1'),
         str(folder), str(binary), str(project)],
        env=environment, creationflags=subprocess.CREATE_NEW_CONSOLE,
        startupinfo=startup)
    attached = False
    try:
        console.wait_for(lambda: (folder / 'ready').exists(), 'host preparado')
        console.K.FreeConsole()
        console.check(console.K.AttachConsole(host.pid))
        attached = True
        before = console.state()
        (folder / 'start').touch()
        console.wait_for(lambda: 'Configuración:' in console.screen(), 'consola iniciada')
        console.wait_for(lambda: 'Elige un número' in console.screen(), 'selección de modelos', seconds=20)
        yield
        command('/exit')
        console.wait_for(lambda: (folder / 'result').exists(), 'salida restaurada')
        after = console.state()
        code = int((folder / 'result').read_text(encoding='utf-8-sig').strip())
        report['sesiones'].append({'codigo': code, 'antes': before, 'despues': after})
        assert code == 0 and before == after, (code, before, after)
    except Exception:
        if attached:
            report['pantalla_fallo'] = console.screen()
        raise
    finally:
        if attached and host.poll() is None and not (folder / 'result').exists():
            console.press('\x03')
            try:
                console.wait_for(lambda: (folder / 'result').exists(), 'salida tras fallo', seconds=15)
            except AssertionError:
                pass
        (folder / 'finish').touch()
        try:
            host.wait(timeout=5)
        except subprocess.TimeoutExpired:
            host.terminate()
            host.wait(timeout=5)
        if attached:
            console.K.FreeConsole()


def run(args, report):
    with tempfile.TemporaryDirectory(prefix='pill-features-') as temporary:
        folder = Path(temporary)
        project = folder / 'Proyecto ñ'
        control = project / '.pillcli'
        control.mkdir(parents=True)
        private = folder / 'privado'
        private.mkdir()
        trust = private / 'PILLCLI/trust.json'
        script = project / 'permission_probe.py'
        script.write_text("from pathlib import Path\nPath('ejecutado.txt').write_text('ejecutado')\n", encoding='utf-8')
        configuration = {
            'ui': {'poll_ms': 40, 'max_history_entries': 256, 'ascii_logo': True},
            'commands': {'commands': [{
                'id': 'consola', 'program': str(Path(args.python).resolve()),
                'args': ['-B', script.name], 'cwd': '.', 'watched_files': [script.name]}]}}
        (control / 'config.json').write_text(json.dumps(configuration), encoding='utf-8')
        captures, checks = report['capturas'], report['checks']
        with session(folder / 'primera', project, args.binary, private, report):
            command('/permissions')
            console.wait_for(lambda: 'Propuestas · consentimiento explícito' in console.screen(), 'lista de permisos')
            captures['permisos_fila_corta'] = console.screen()
            assert 'command:consola' in captures['permisos_fila_corta']
            assert script.name not in captures['permisos_fila_corta']
            assert not trust.exists()
            console.press(vk=0x0D)
            console.wait_for(lambda: script.name in console.screen(), 'detalle con Enter')
            captures['permisos_detalle'] = console.screen()
            checks['enter_muestra_detalle_sin_conceder'] = not trust.exists()
            console.press(vk=0x0D, control=0x0008)
            console.wait_for(lambda: 'Comando concedido' in console.screen(), 'concesión explícita')
            checks['ctrl_enter_concede'] = trust.exists()
            captures['concesion'] = console.screen()
            command('/permissions')
            console.wait_for(lambda: 'Propuestas · consentimiento explícito' in console.screen(), 'permisos concedidos')
            console.press(vk=0x0D, control=0x0008)
            console.wait_for(lambda: 'Concesión retirada' in console.screen(), 'revocación explícita')
            grants = json.loads(trust.read_text(encoding='utf-8'))
            checks['ctrl_enter_retira'] = all(not values for values in grants['projects'].values())
            checks['conceder_no_ejecuta'] = not (project / 'ejecutado.txt').exists()
            console.press(vk=0x1B)

            for label, grapheme in [('marca', 'e\u0301'), ('familia_zwj', '👨‍👩‍👧‍👦'), ('bandera', '🇪🇸')]:
                console.press('A' + grapheme + 'Z')
                console.press(vk=0x25)
                console.press(vk=0x08)
                console.wait_for(lambda: draft('AZ'), 'Retroceso de grafema ' + label)
                captures['grafema_' + label] = console.screen()
                checks['retroceso_' + label] = True
                clear_line(2)

            console.press('uno dos')
            console.press(vk=0x24)
            console.press(vk=0x27, control=0x0008)
            console.press('X')
            console.wait_for(lambda: draft('unoX dos'), 'Ctrl+derecha alcanza fin de palabra')
            checks['salto_palabra'] = True
            clear_line(8)
            console.press('primera')
            console.press(vk=0x0D)
            console.press('segunda')
            console.press(vk=0x24)
            console.press('X')
            console.press(vk=0x26)
            console.press(vk=0x24)
            console.press('Y')
            console.press(vk=0x28)
            console.press(vk=0x2E)
            console.wait_for(lambda: draft('Yprimera') and '  Xegunda' in console.screen().splitlines(), 'edición en dos líneas')
            captures['borrador_multilinea'] = console.screen()
            checks['enter_linea_inicio_arriba_abajo'] = True
            console.press(vk=0x26)
            clear_line(len('Yprimera\nXegunda'))

            console.press(vk=0x52, control=0x0008)
            console.wait_for(lambda: 'No hay razonamiento en esta respuesta.' in console.screen(), 'Ctrl+R sin respuesta')
            checks['ctrl_r_sin_razonamiento'] = True
            for _ in range(4):
                command('/help')
            console.wait_for(lambda: 'hay texto por encima' in console.screen(), 'historial con desplazamiento')
            console.press(vk=0x21)
            console.wait_for(lambda: 'hay texto por debajo' in console.screen(), 'PgUp')
            captures['pagina_arriba'] = console.screen()
            console.press('BORRADOR')
            console.wait_for(lambda: draft('BORRADOR') and 'hay texto por debajo' in console.screen(), 'escribir conserva desplazamiento')
            checks['escribir_conserva_scroll'] = True
            for _ in range(20):
                console.press(vk=0x22)
            console.wait_for(lambda: 'hay texto por debajo' not in console.screen(), 'PgDn regresa al final')
            checks['paginacion'] = True
            clear_line(len('BORRADOR'))
            command('/task')
            console.wait_for(lambda: 'Modo: Tarea' in console.screen(), 'cabecera modo tarea')
            command('/chat')
            console.wait_for(lambda: 'Modo: Chat' in console.screen(), 'cabecera modo chat')
            checks['modos_visibles'] = True
            command('/journal')
            console.wait_for(lambda: 'No hay diario en este proyecto.' in console.screen(), 'diario ausente')
            checks['journal_no_crea_diario'] = not list((control / 'sessions').glob('*.jsonl'))
            captures['diario_ausente'] = console.screen()

        saved = json.loads((control / 'console.json').read_text(encoding='utf-8'))
        checks['historial_guardado_por_producto'] = any('No hay diario en este proyecto.' in line for line in saved['lines'])
        # Fallo real de formato en un diario descartable, no una respuesta ficticia.
        broken = control / 'sessions/interrumpida.jsonl'
        broken.write_bytes(b'{')
        before_hash = hashlib.sha256(broken.read_bytes()).hexdigest()
        report['fallo_inyectado'] = {'archivo': '.pillcli/sessions/interrumpida.jsonl', 'bytes_hex': '7b', 'metodo': 'escritura de un JSON truncado en muestra temporal'}
        with session(folder / 'segunda', project, args.binary, private, report):
            captures['reinicio_historial'] = console.screen()
            command('/permissions')
            console.wait_for(lambda: 'Propuestas · consentimiento explícito' in console.screen(), 'lista tras reinicio')
            console.press(vk=0x1B)
            captures['reinicio_recuperacion'] = console.screen()
            console.wait_for(lambda: 'Efectos parciales:' in console.screen(), 'bloqueo persistente visible')
            checks['recuperacion_desconocida_visible'] = True
            command('/journal')
            console.wait_for(lambda: 'Diario interrumpida.jsonl' in console.screen(), 'lectura de diario truncado')
            captures['journal_no_reproduce_efectos'] = console.screen()
            checks['journal_conserva_aviso'] = 'Efectos parciales:' in console.screen()
            checks['journal_no_cambia_archivo'] = hashlib.sha256(broken.read_bytes()).hexdigest() == before_hash
            checks['journal_no_crea_otro'] = len(list((control / 'sessions').glob('*.jsonl'))) == 1
            history_visible = 'Historial local de la consola' in console.screen()
            for index in range(20):
                if history_visible:
                    break
                console.press(vk=0x21)
                console.wait_for(lambda: 'hay texto por debajo' in console.screen(), 'historial anterior tras reinicio')
                captures['historial_recuperado_pagina_' + str(index)] = console.screen()
                history_visible = 'Historial local de la consola' in console.screen()
            checks['historial_recuperado'] = history_visible or any('Historial local de la consola' in capture for capture in captures.values())
            for _ in range(20):
                console.press(vk=0x22)
        checks['ningun_script_ejecutado'] = not (project / 'ejecutado.txt').exists()
        assert all(checks.values()), checks


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=console.ROOT / 'bin/pill.exe')
    parser.add_argument('--python', required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    report = {'fecha_utc': datetime.now(timezone.utc).isoformat(),
              'sha256_binario': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'alcance': 'Consola Win32 real sin inferencia. Diario truncado explícito en muestra temporal; no acredita razonamiento de modelo ni recuperación de una caída real.',
              'checks': {}, 'capturas': {}, 'sesiones': [], 'resultado': 'FAIL'}
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    try:
        run(args, report)
        report['resultado'] = 'PASS'
    except Exception as error:
        report['error'] = repr(error)
    args.evidence.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(report['resultado'] + ': funciones de consola, permisos y recuperación; evidencia en ' + str(args.evidence))
    return 0 if report['resultado'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
