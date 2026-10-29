"""UI Windows real: cabecera compacta, selección, permisos y vistas de revisión.

Solo metadatos de Ollama: no se envían peticiones de generación ni se ejecuta
ningún comando concedido. Todos los archivos y concesiones de prueba son
temporales. La segunda sesión inyecta un diario truncado, identificado como
fallo de diagnóstico, para comprobar lectura y revisión humana durables.
"""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import urllib.parse
import urllib.request
import winreg

import test_console as console


def digest(data):
    return hashlib.sha256(data).hexdigest()


def user_path_signature():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, 'Environment') as key:
            value, kind = winreg.QueryValueEx(key, 'Path')
    except FileNotFoundError:
        value, kind = None, None
    return digest(json.dumps([value, kind], ensure_ascii=True).encode('utf-8'))


def installed_model(url, name):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != 'http' or parsed.hostname not in ('localhost', '127.0.0.1'):
        raise ValueError('la sonda admite únicamente metadatos HTTP loopback')

    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, request, fp, code, message, headers, target):
            raise RuntimeError('la sonda no sigue redirecciones')

    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(url.rstrip('/') + '/api/tags', timeout=15) as response:
        models = json.loads(response.read())['models']
    return next(model for model in models if model['name'] == name)


def command(text):
    console.press(text)
    console.press(vk=0x0D)


def capture(report, label, predicate, seconds=20):
    def ready():
        frame = console.screen()
        return frame if predicate(frame) else False
    frame = console.wait_for(ready, label, seconds=seconds)
    report['captures'][label] = frame
    return frame


def footer(frame):
    lines = frame.splitlines()
    return lines[-1] if lines else ''


def marked_rows(frame):
    return [line for line in frame.splitlines() if re.match(r'^>\s+\d+\s+·', line)]


VIEWPORT_TRACE = []


def set_viewport(columns, rows):
    """Redimensiona nuestro HWND; propaga el tamaño a ambos buffers VT."""
    class PixelRect(console.c.Structure):
        _fields_ = [('left', console.w.LONG), ('top', console.w.LONG),
                    ('right', console.w.LONG), ('bottom', console.w.LONG)]

    class FontInfo(console.c.Structure):
        _fields_ = [('size', console.w.ULONG), ('font', console.w.DWORD),
                    ('cell', console.Coord), ('family', console.w.UINT),
                    ('weight', console.w.UINT), ('face', console.w.WCHAR * 32)]

    console.U.GetWindowRect.argtypes = [console.w.HWND, console.c.POINTER(PixelRect)]
    console.U.GetWindowRect.restype = console.w.BOOL
    console.K.GetCurrentConsoleFontEx.argtypes = [console.w.HANDLE, console.w.BOOL, console.c.POINTER(FontInfo)]
    console.K.GetCurrentConsoleFontEx.restype = console.w.BOOL
    hwnd = console.K.GetConsoleWindow()
    assert hwnd, 'ventana propia de consola disponible'
    for attempt in range(12):
        observed = console.dimensions()
        if observed == [columns, rows]:
            return
        rect = PixelRect()
        console.check(console.U.GetWindowRect(hwnd, console.c.byref(rect)))
        output = console.handle('CONOUT$')
        try:
            font = FontInfo()
            font.size = console.c.sizeof(font)
            console.check(console.K.GetCurrentConsoleFontEx(output, False, console.c.byref(font)))
        finally:
            console.K.CloseHandle(output)
        assert font.cell.X > 0 and font.cell.Y > 0
        width = rect.right - rect.left + (columns - observed[0]) * font.cell.X
        height = rect.bottom - rect.top + (rows - observed[1]) * font.cell.Y
        VIEWPORT_TRACE.append({'requested': [columns, rows], 'observed': observed,
                               'font_cell': [font.cell.X, font.cell.Y],
                               'window_rect': [rect.left, rect.top, rect.right, rect.bottom],
                               'resize_pixels': [width, height], 'attempt': attempt})
        console.check(console.U.SetWindowPos(hwnd, None, 0, 0, width, height, 0x0016))
        try:
            console.wait_for(lambda: console.dimensions() != observed, 'resize de ventana propia', seconds=1)
        except AssertionError:
            pass
    raise AssertionError(f'viewport solicitado {columns}x{rows}, observado {console.dimensions()}; ver trace')


def assert_frame_visible(frame, dimensions):
    assert console.dimensions() == list(dimensions), (console.dimensions(), dimensions)
    assert len(frame.splitlines()) <= dimensions[1]
    assert all(len(line) < dimensions[0] for line in frame.splitlines())
    assert any(line.startswith('>') for line in frame.splitlines()), 'cursor de entrada visible'
    output = console.handle('CONOUT$')
    try:
        info = console.Info()
        console.check(console.K.GetConsoleScreenBufferInfo(output, console.c.byref(info)))
        assert info.window.Left <= info.cursor.X <= info.window.Right
        assert info.window.Top <= info.cursor.Y <= info.window.Bottom
    finally:
        console.K.CloseHandle(output)


def recovery_progress(frame):
    found = re.search(r'Recuperación\s+·\s+Pendientes:\s*(\d+)\s+·\s+Revisadas:\s*(\d+)\s+·\s+Total:\s*(\d+)', frame)
    return tuple(map(int, found.groups())) if found else None


def review_rows(frame):
    return [(match[1], match[2]) for line in frame.splitlines()
            if (match := re.match(r'^[ >]+\d+\s+·\s+(pendiente|revisada|sin revisión pendiente)\s+·(.*)', line))]


def header_contains(frame, value):
    return any(value in line for line in frame.splitlines()[:3])


def close_detail(report, label):
    console.press(vk=0x1B)
    return capture(report, label, lambda frame: 'Esc volver' not in footer(frame))


def inspect_detail(report, label, needle):
    seen = []
    for index in range(20):
        frame = console.screen()
        report['captures'][f'{label}_{index}'] = frame
        seen.append(frame)
        if needle in frame:
            return frame
        console.press(vk=0x22)
        try:
            console.wait_for(lambda: console.screen() != frame, 'desplazamiento de detalle', seconds=2)
        except AssertionError:
            break
    raise AssertionError(f'{needle!r} no aparece en el detalle {label}: {seen!r}')


@contextmanager
def session(folder, project, binary, private, report, viewport=None):
    folder.mkdir()
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    environment = dict(os.environ, LOCALAPPDATA=str(private))
    host = subprocess.Popen(
        [shutil.which('pwsh'), '-NoProfile', '-File',
         str(console.ROOT / 'diagnostics/console_host.ps1'),
         str(folder), str(binary), str(project)],
        env=environment, creationflags=subprocess.CREATE_NEW_CONSOLE,
        startupinfo=startup)
    attached = False
    try:
        console.wait_for(lambda: (folder / 'ready').exists(), 'host propio')
        console.K.FreeConsole()
        console.check(console.K.AttachConsole(host.pid))
        attached = True
        if viewport:
            set_viewport(*viewport)
        before = console.state()
        (folder / 'start').touch()
        capture(report, folder.name + '_startup', lambda frame: 'Elige un número' in frame and '/ comandos' in frame)
        yield
        console.press('\x03')
        console.wait_for(lambda: (folder / 'result').exists(), 'salida y restauración')
        code = int((folder / 'result').read_text(encoding='utf-8-sig').strip())
        after = console.state()
        assert code == 0 and before == after, (code, before, after)
        report['sessions'].append({'host_pid': host.pid, 'exit_code': code, 'before': before, 'after': after})
    except BaseException:
        if attached:
            report['failure_screen'] = console.screen()
        raise
    finally:
        if attached and host.poll() is None and not (folder / 'result').exists():
            console.press('\x03')
            try:
                console.wait_for(lambda: (folder / 'result').exists(), 'cierre tras fallo', seconds=15)
            except AssertionError:
                pass
        (folder / 'finish').touch()
        try:
            host.wait(timeout=5)
        except subprocess.TimeoutExpired:
            # Solo el host Popen creado y retenido por esta sonda.
            host.terminate()
            host.wait(timeout=5)
        if attached:
            console.K.FreeConsole()


def run(args, report):
    binary = Path(args.binary).resolve()
    expected_model = installed_model(args.ollama_url, args.model)
    report['selected_metadata'] = expected_model
    with tempfile.TemporaryDirectory(prefix='pill-ui-improvements-') as temporary:
        base = Path(temporary)
        project = base / 'Proyecto ñ'
        control = project / '.pillcli'
        control.mkdir(parents=True)
        private = base / 'private'
        private.mkdir()
        trust = private / 'PILLCLI/trust.json'
        original = b'original\r\n'
        (project / 'base.txt').write_bytes(original)
        script = project / 'probe.py'
        script.write_text("from pathlib import Path\nPath('executed.txt').write_text('ejecutado')\n", encoding='utf-8')
        config = {
            'ui': {'ascii_logo': True, 'compact_header': True, 'poll_ms': 40},
            'ollama': {'url': args.ollama_url, 'model': None},
            'gpu': {'exclusive': False},
            'profiles': {'auditoria-ui': {'num_ctx': 8192, 'num_predict': 512, 'temperature': 0.2, 'think': False}},
            'commands': {'commands': [{'id': 'consola', 'program': str(Path(args.python).resolve()),
                                     'args': ['-B', script.name], 'cwd': '.', 'watched_files': [script.name]}]}}
        config_path = control / 'config.json'
        config_path.write_text(json.dumps(config), encoding='utf-8')
        config_hash = digest(config_path.read_bytes())
        checks = report['checks']

        with session(base / 'first', project, binary, private, report):
            frame = console.screen()
            assert '(###|   )' in frame.splitlines()[0]
            assert 'Proyecto ñ' in frame.splitlines()[0]
            checks['compact_capsule_and_project'] = True
            choices = [line for line in frame.splitlines() if args.model in line and ' · ' in line]
            assert choices, (args.model, frame)
            number = choices[0].strip().lstrip('> ').split(' · ')[0].strip()
            assert number.isdigit(), (number, frame)
            console.press(number)
            console.press(vk=0x0D, control=0x0008)
            capture(report, 'selected', lambda value: header_contains(value, 'Preparado · Editar'))
            command('/model')
            capture(report, 'model_selector', lambda value: 'Ctrl+Enter elegir' in footer(value) and marked_rows(value))
            console.press(vk=0x1B)
            escaped = capture(report, 'model_escape', lambda value: not marked_rows(value) and 'Ctrl+Enter elegir' not in footer(value))
            assert args.model[:20] in '\n'.join(escaped.splitlines()[:3])
            checks['escape_closes_models_preserving_selected'] = True

            command('/permissions')
            frame = capture(report, 'permissions', lambda value: 'conceder/retirar' in footer(value))
            assert 'Ejecutar prueba/comando consola' in frame
            assert 'Enter detalle' in footer(frame) and 'Enter línea' not in footer(frame)
            console.press(vk=0x0D)
            capture(report, 'permission_detail', lambda value: 'Esc volver' in footer(value))
            inspect_detail(report, 'permission_program', str(Path(args.python).resolve()))
            assert not trust.exists()
            checks['permission_enter_only_opens_readable_detail'] = True
            close_detail(report, 'permission_detail_closed')
            console.press(vk=0x0D, control=0x0008)
            capture(report, 'permission_granted', lambda value: header_contains(value, 'Comando concedido'))
            assert trust.exists()
            command('/permissions')
            capture(report, 'permission_reopen', lambda value: 'conceder/retirar' in footer(value))
            console.press(vk=0x0D, control=0x0008)
            capture(report, 'permission_revoked', lambda value: header_contains(value, 'Concesión retirada'))
            grants = json.loads(trust.read_text(encoding='utf-8'))
            assert all(not values for values in grants['projects'].values())
            console.press(vk=0x1B)
            checks['ctrl_enter_grants_and_revokes_only_the_fixture'] = True
            assert not (project / 'executed.txt').exists()

            for mode, label in [('/ask', 'Preguntar'), ('/edit', 'Editar'), ('/task', 'Tarea'), ('/chat', 'Editar')]:
                command(mode)
                capture(report, 'mode_' + mode[1:], lambda value, label=label: header_contains(value, label) and not marked_rows(value))
            checks['ask_edit_task_and_chat_alias_are_visible'] = True

            command('/status')
            capture(report, 'status', lambda value: 'Estado de la sesión' in value and 'Esc volver' in footer(value))
            inspect_detail(report, 'status_model', expected_model['digest'])
            checks['status_exposes_selected_digest_from_real_metadata'] = True
            close_detail(report, 'status_closed')

            command('/profile')
            frame = capture(report, 'profiles', lambda value: 'Ctrl+Enter aplicar perfil' in footer(value))
            choice = next(line for line in frame.splitlines() if 'auditoria-ui' in line)
            number = choice.strip().lstrip('> ').split(' · ')[0].strip()
            console.press(number)
            console.press(vk=0x0D)
            capture(report, 'profile_detail', lambda value: 'Esc volver' in footer(value))
            original_viewport = console.dimensions()
            set_viewport(73, 21)
            profile_small = capture(report, 'profile_detail_73x21', lambda value: 'Ctrl+Enter aplicar perfil' in footer(value))
            assert_frame_visible(profile_small, (73, 21))
            checks['profile_detail_73x21_shows_real_apply_shortcut'] = True
            set_viewport(*original_viewport)
            inspect_detail(report, 'profile_parameters', '8192')
            close_detail(report, 'profile_detail_closed')
            console.press(vk=0x0D, control=0x0008)
            capture(report, 'profile_applied', lambda value: 'Perfil seleccionado' in value and 'Esc volver' in footer(value))
            inspect_detail(report, 'profile_applied_parameters', '8192')
            close_detail(report, 'profile_applied_closed')
            checks['profile_is_explicit_without_model_change_or_inference'] = True

            for name, text in [('/recover', 'Recuperación:'), ('/changes', 'Cambios observados:'), ('/undo', 'Restaurar:'), ('/archive', 'Archivo:')]:
                command(name)
                capture(report, 'view_' + name[1:], lambda value, text=text: text in value)
                console.press(vk=0x1B)
            checks['review_changes_undo_and_archive_views_are_reachable'] = True
            assert digest(config_path.read_bytes()) == config_hash
            assert (project / 'base.txt').read_bytes() == original
            assert not (project / 'executed.txt').exists()
            checks['previews_do_not_edit_run_or_change_project_configuration'] = True

            command('/copy')
            capture(report, 'copy_without_answer', lambda value: 'No hay respuesta completa para copiar.' in value)
            checks['copy_without_confirmed_answer_reports_absence'] = True
            # No se modifica el portapapeles global del usuario en esta sonda.
            for index in range(20):
                console.press(f'línea-{index:02}')
                if index != 19:
                    console.press(vk=0x0D)
            capture(report, 'long_draft', lambda value: 'línea-19' in value and '/ comandos' in footer(value))
            old_size = console.dimensions()
            console.check(console.U.SetWindowPos(console.K.GetConsoleWindow(), None, 0, 0, 720, 440, 0x0016))
            small = capture(report, 'compact_resize', lambda value: console.dimensions()[0] < old_size[0] and 'línea-19' in value and '/ comandos' in footer(value))
            assert all(len(line) < console.dimensions()[0] for line in small.splitlines())
            checks['compact_resize_keeps_cursor_draft_and_footer'] = True

        sessions = control / 'sessions'
        sessions.mkdir(exist_ok=True)
        broken = sessions / 'interrumpida.jsonl'
        broken.write_bytes(b'{"session":"interrumpida","stage":"requested"')
        before_broken = digest(broken.read_bytes())
        report['injected_fault'] = {'kind': 'diario truncado en muestra descartable', 'sha256': before_broken}
        with session(base / 'second', project, binary, private, report, viewport=(73, 21)):
            capture(report, 'recovery_banner', lambda value: any(line.startswith('Efectos parciales:') for line in value.splitlines()))
            console.press(vk=0x1B)
            command('/recover')
            recovery_rows = capture(report, 'recovery_rows', lambda value: 'Ctrl+Enter confirmar fila' in footer(value) and recovery_progress(value) == (2, 0, 2))
            assert_frame_visible(recovery_rows, (73, 21))
            rows = review_rows(recovery_rows)
            assert len(rows) == 2 and all(state == 'pendiente' and 'unknown' in text for state, text in rows)
            checks['recovery_73x21_counts_and_labels_two_pending_unknown_entries'] = True
            review_count = len([line for line in recovery_rows.splitlines() if re.match(r'^[ >]+\d+\s+·', line)])
            assert review_count == 2, 'el fallo inyectado produce terminador ausente y JSON incompleto'
            console.press(vk=0x0D)
            recovery_detail = capture(report, 'recovery_evidence', lambda value: 'Esc volver' in footer(value) and recovery_progress(value) == (2, 0, 2))
            assert_frame_visible(recovery_detail, (73, 21))
            assert any(line.startswith('Efectos parciales:') for line in recovery_detail.splitlines())
            checks['recovery_detail_keeps_counter_banner_cursor_and_footer_73x21'] = True
            assert digest(broken.read_bytes()) == before_broken
            close_detail(report, 'recovery_back')
            checks['unknown_recovery_is_visible_and_enter_does_not_acknowledge'] = True
            for index in range(review_count):
                if index:
                    console.press(str(index + 1))
                    console.press(vk=0x0D)
                    capture(report, f'recovery_evidence_{index+1}', lambda value: 'Esc volver' in footer(value))
                    close_detail(report, f'recovery_back_{index+1}')
                console.press(vk=0x0D, control=0x0008)
                review_path = control / 'recovery.json'
                def review_written():
                    try:
                        return len(json.loads(review_path.read_text(encoding='utf-8'))['fingerprints']) >= index+1
                    except (FileNotFoundError, json.JSONDecodeError):
                        return False
                console.wait_for(review_written, 'revision durable de cada evidencia')
                def reviewed_frame(value):
                    blocked = any(line.startswith('Efectos parciales:') for line in value.splitlines())
                    return ('Ctrl+Enter confirmar fila' in footer(value)
                            and blocked == (index+1 < review_count)
                            and not header_contains(value, 'Registrando')
                            and recovery_progress(value) == (review_count-index-1, index+1, review_count))
                reviewed = capture(report, f'recovery_reviewed_{index+1}', reviewed_frame)
                assert_frame_visible(reviewed, (73, 21))
                rows = review_rows(reviewed)
                assert len(rows) == review_count
                assert sum(state == 'revisada' for state, _ in rows) == index+1
                assert sum(state == 'pendiente' for state, _ in rows) == review_count-index-1
                assert all('unknown' in text for _, text in rows)
                assert digest(broken.read_bytes()) == before_broken
            checks['human_review_updates_counts_and_labels_preserving_unknown_history'] = True
            checks['reviewing_one_entry_does_not_clear_other_unknown_entries'] = True
            assert digest(broken.read_bytes()) == before_broken
            assert (project / 'base.txt').read_bytes() == original
            assert not (project / 'executed.txt').exists()
            checks['human_review_is_durable_without_replaying_or_rewriting_evidence'] = True
        with session(base / 'third', project, binary, private, report, viewport=(73, 21)):
            frame = console.screen()
            assert not any(line.startswith('Efectos parciales:') for line in frame.splitlines())
            assert digest(broken.read_bytes()) == before_broken
            checks['restart_keeps_acknowledgment_and_preserves_unknown_evidence'] = True
            console.press(vk=0x1B)
            command('/recover')
            restored = capture(report, 'reviewed_recovery_after_restart_73x21', lambda value: recovery_progress(value) == (0, 2, 2) and 'Ctrl+Enter confirmar fila' in footer(value))
            assert_frame_visible(restored, (73, 21))
            rows = review_rows(restored)
            assert len(rows) == 2 and all(state == 'revisada' and 'unknown' in text for state, text in rows)
            assert not any(line.startswith('Efectos parciales:') for line in restored.splitlines())
            assert digest(broken.read_bytes()) == before_broken
            checks['restart_restores_reviewed_labels_and_zero_pending_counter_73x21'] = True


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--python', required=True)
    parser.add_argument('--model', required=True)
    parser.add_argument('--ollama-url', default=json.loads((console.ROOT / 'config/defaults.json').read_text(encoding='utf-8'))['ollama']['url'])
    parser.add_argument('--binary', type=Path, default=console.ROOT / 'bin/pill.exe')
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    if args.evidence.exists():
        raise FileExistsError('No se sobrescriben evidencias; elige otro nombre.')
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    path_before = user_path_signature()
    report = {'fecha_utc': datetime.now(timezone.utc).isoformat(), 'result': 'FAIL',
              'binary_sha256': digest(args.binary.read_bytes()), 'captures': {}, 'checks': {}, 'sessions': [],
              'scope': 'Consola Win32 real, metadatos Ollama reales, sin inferencia ni comandos ejecutados; concesión temporal del fixture; PATH persistente sin cambios.'}
    try:
        run(args, report)
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        report['viewport_resize_trace'] = VIEWPORT_TRACE
        report['checks']['user_path_preserved'] = user_path_signature() == path_before
        report['result'] = 'PASS' if report['checks'] and all(report['checks'].values()) and 'error' not in report else 'FAIL'
        args.evidence.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({'result': report['result'], 'checks': report['checks'], 'error': report.get('error')}, ensure_ascii=True))
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
