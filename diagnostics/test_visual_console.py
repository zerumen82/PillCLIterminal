"""Diff y jerarquia visual en consolas Win32 reales e inferencia local explicita.

Dos proyectos temporales independientes (color y monocromo), un modelo a la
vez, herramientas reales y comprobacion de disco/diario/respaldo. No escribe
User/Machine PATH, no toca portapapeles y no descarga modelos ni libera GPU.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

import test_console as c
import test_ui_improvements as ui


ORIGINAL = (b'# Contexto de la prueba visual\n'
            b'def identidad(x):\n'
            b'    return x\n'
            b'\n'
            b'def suma(a, b):\n'
            b'    return a - b\n'
            b'\n'
            b'# Contexto final conservado\n')
EXPECTED = ORIGINAL.replace(b'a - b', b'a + b')
VERIFY = "import calc\nassert calc.suma(2, 3) == 5\nassert calc.suma(-2, 3) == 1\nprint('VISUAL_SOURCE_PASS')\n"
PROMPT = ('Corrige calc.py: suma debe sumar a y b. Lee calc.py con read_file, '
          'usa edit_file con el SHA real y cambia solamente a - b por a + b. '
          'Vuelve a leer calc.py para comprobar los bytes resultantes. '
          'No crees otros archivos ni cambies verify.py. Responde brevemente al terminar.')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def read_frame():
    """Texto y atributos reales del buffer activo, coordinados por celda."""
    c.K.ReadConsoleOutputAttribute.argtypes = [c.w.HANDLE, c.c.POINTER(c.w.WORD), c.w.DWORD,
                                              c.Coord, c.c.POINTER(c.w.DWORD)]
    c.K.ReadConsoleOutputAttribute.restype = c.w.BOOL
    output = c.handle('CONOUT$')
    try:
        info = c.Info()
        c.check(c.K.GetConsoleScreenBufferInfo(output, c.c.byref(info)))
        width = info.window.Right - info.window.Left + 1
        height = info.window.Bottom - info.window.Top + 1
        rows = []
        for y in range(info.window.Top, info.window.Bottom + 1):
            text = c.c.create_unicode_buffer(width + 1)
            count = c.w.DWORD()
            c.check(c.K.ReadConsoleOutputCharacterW(output, text, width,
                                                   c.Coord(info.window.Left, y), c.c.byref(count)))
            attributes = (c.w.WORD * width)()
            c.check(c.K.ReadConsoleOutputAttribute(output, attributes, width,
                                                   c.Coord(info.window.Left, y), c.c.byref(count)))
            rows.append({'text': text.value.rstrip(), 'attributes': list(attributes)})
        return {'text': '\n'.join(row['text'] for row in rows).rstrip(),
                'rows': rows, 'dimensions': [width, height],
                'cursor': [info.cursor.X - info.window.Left, info.cursor.Y - info.window.Top]}
    finally:
        c.K.CloseHandle(output)


def capture(case, label):
    frame = read_frame()
    case['captures'][label] = frame
    return frame


def visible_bounds(frame):
    assert frame['dimensions'] == [73, 21], frame['dimensions']
    assert len(frame['text'].splitlines()) <= 21
    assert all(len(line) < 73 for line in frame['text'].splitlines())
    assert 0 <= frame['cursor'][0] < 73 and 0 <= frame['cursor'][1] < 21
    assert any(line.startswith('>') for line in frame['text'].splitlines())


def command(value):
    c.press(value)
    c.press(vk=0x0D)


def wait_complete(case, timeout):
    started = time.monotonic()
    last = None
    while time.monotonic() - started < timeout:
        screen = c.screen()
        if screen != last:
            case['operation_screens'].append(screen)
            last = screen
        status = '\n'.join(screen.splitlines()[:3])
        if 'Respuesta completa' in status:
            case['elapsed_seconds'] = time.monotonic() - started
            return
        if 'Operación fallida' in status or 'Cancelado ·' in status:
            raise AssertionError('el producto no confirmo la operacion: ' + status)
        time.sleep(.08)
    c.press(vk=0x1B)
    raise AssertionError('timeout de inferencia; no cuenta como PASS de UI')


def page_history(case):
    pages = []
    for _ in range(32):
        current = c.screen()
        pages.append(current)
        c.press(vk=0x21)
        try:
            c.wait_for(lambda: c.screen() != current, 'pagina anterior', seconds=.3)
        except AssertionError:
            break
    case['history_pages'] = pages
    for _ in range(32):
        c.press(vk=0x22)
    return '\n'.join(pages)


def collect_diff(case, color, default_foreground):
    pages = []
    found = {'added': False, 'removed': False, 'hunk': False, 'context': False}
    metadata_seen = False
    for index in range(48):
        frame = capture(case, f'diff_page_{index}')
        visible_bounds(frame)
        pages.append(frame['text'])
        if 'SHA de base:' in frame['text'] or 'Respaldo:' in frame['text']:
            metadata_seen = True
        for row in frame['rows']:
            text = row['text']
            if '@@ -' in text:
                assert metadata_seen, 'los metadatos deben preceder al diff'
                found['hunk'] = True
            marker = None
            if text.startswith('+') and 'return a + b' in text:
                marker, expected_color = 'added', 2
            elif text.startswith('-') and 'return a - b' in text:
                marker, expected_color = 'removed', 4
            if marker:
                actual = row['attributes'][0] & 0x0F
                case['diff_attributes'].append({'kind': marker, 'text': text,
                                                'foreground': actual, 'color_enabled': color})
                assert actual == (expected_color if color else default_foreground), (text, actual)
                found[marker] = True
            if text.startswith(' ') and ('def suma' in text or '# Contexto final' in text):
                found['context'] = True
        previous = frame['text']
        c.press(vk=0x22)
        try:
            c.wait_for(lambda: c.screen() != previous, 'pagina siguiente del diff', seconds=.4)
        except AssertionError:
            break
    joined = '\n'.join(pages)
    assert all(found.values()), found
    assert metadata_seen and 'verified' in joined
    case['checks']['diff_ranges_markers_context_visible'] = True
    case['checks']['diff_color_matches_configuration'] = True
    return joined


def disk_evidence(project, case):
    journals = []
    for path in sorted((project / '.pillcli/sessions').glob('*.jsonl')):
        records = [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines() if line]
        journals.append({'name': path.name, 'sha256': digest(path.read_bytes()), 'records': records})
    case['journals'] = journals
    observed = [record for journal in journals for record in journal['records']
                if record.get('stage') == 'observed']
    edits = [record for record in observed
             if record.get('tool') in ('edit_file', 'edit_file_many') and record.get('status') == 'verified']
    reads = [record for record in observed
             if record.get('tool') == 'read_file' and record.get('status') == 'verified']
    assert edits, 'edicion no observada por el ejecutor'
    assert any(record.get('sha256') == digest(ORIGINAL) for record in reads)
    assert any(record.get('sha256') == digest(EXPECTED) for record in reads)
    assert edits[-1]['sha256'] == digest(EXPECTED)
    case['backups'] = []
    for edit in edits:
        path = Path(edit['backup_path'])
        raw = path.read_bytes()
        case['backups'].append({'path': str(path), 'bytes_hex': raw.hex(), 'sha256': digest(raw)})
    assert any(item['sha256'] == digest(ORIGINAL) for item in case['backups'])
    case['checks']['native_read_edit_reread_and_backup_are_real'] = True
    return edits[-1]


def run_case(args, parent, color):
    label = 'color' if color else 'monocromo'
    case = {'name': label, 'color': color, 'checks': {}, 'captures': {},
            'operation_screens': [], 'diff_attributes': [], 'result': 'FAIL'}
    parent['cases'].append(case)
    host = None
    attached = False
    print('RUN ' + label, flush=True)
    with tempfile.TemporaryDirectory(prefix='pill-visual-' + label + '-') as temporary:
        folder = Path(temporary)
        project = folder / 'Proyecto'
        control = project / '.pillcli'
        control.mkdir(parents=True)
        private = folder / 'private'
        private.mkdir()
        source = project / 'calc.py'
        source.write_bytes(ORIGINAL)
        verify = project / 'verify.py'
        verify.write_text(VERIFY, encoding='utf-8')
        test_hash = digest(verify.read_bytes())
        config = {'ui': {'color': color, 'ascii_logo': True, 'compact_header': True,
                         'poll_ms': 40, 'max_history_entries': 256},
                  'ollama': {'url': args.ollama_url, 'model': args.model,
                             'num_ctx': 16384, 'num_predict': 2048, 'think': True},
                  'gpu': {'exclusive': False},
                  'files': {'excluded_components': ['target', 'node_modules', 'venv', '.venv', 'verify.py']}}
        config_path = control / 'config.json'
        config_path.write_text(json.dumps(config), encoding='utf-8')
        config_hash = digest(config_path.read_bytes())
        before_test = subprocess.run([args.python, '-B', verify.name], cwd=project,
                                     capture_output=True, text=True, timeout=15)
        assert before_test.returncode != 0
        case['test_before'] = {'exit_code': before_test.returncode, 'stdout': before_test.stdout,
                               'stderr': before_test.stderr}
        startup = subprocess.STARTUPINFO()
        startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
        try:
            host = subprocess.Popen([shutil.which('pwsh'), '-NoProfile', '-File',
                                     str(c.ROOT / 'diagnostics/console_host.ps1'),
                                     str(folder), str(args.binary), str(project)],
                                    env=dict(os.environ, LOCALAPPDATA=str(private)),
                                    creationflags=subprocess.CREATE_NEW_CONSOLE, startupinfo=startup)
            c.wait_for(lambda: (folder / 'ready').exists(), 'host propio')
            c.K.FreeConsole()
            c.check(c.K.AttachConsole(host.pid))
            attached = True
            ui.set_viewport(73, 21)
            before_state = c.state()
            (folder / 'start').touch()
            c.wait_for(lambda: 'Preparado' in c.screen(), 'modelo explicito preparado', seconds=45)
            capture(case, 'startup')
            c.press(PROMPT)
            c.press(vk=0x0D, control=0x0008)
            wait_complete(case, args.timeout)
            case['checks']['operation_confirmed_by_product'] = True
            assert source.read_bytes() == EXPECTED, source.read_bytes()
            last_edit = disk_evidence(project, case)
            transcript = json.loads((control / 'model.json').read_text(encoding='utf-8'))
            assert transcript['model'] == args.model
            case['transcript'] = transcript
            saved = json.loads((control / 'console.json').read_text(encoding='utf-8'))
            case['saved_console'] = saved
            assert any(line.startswith('\nTú:\n') and PROMPT in line for line in saved['lines'])
            assert any(line.startswith('\nRespuesta:\n') for line in saved['lines'])
            assert any(line.startswith('Acción [') and 'edit_file' in line for line in saved['lines'])
            pages = page_history(case)
            assert all(label in pages for label in ('Tú:', 'Respuesta:', 'Acción [')), pages
            case['checks']['conversation_separates_user_answer_and_actions'] = True
            case['checks']['saved_console_preserves_presentation_headers'] = True
            command('/changes')
            c.wait_for(lambda: bool(re.search(r'^>\s+1\s+·.*calc.py.*verified', c.screen(), re.M)), 'cambio observado')
            c.press(vk=0x0D)
            c.wait_for(lambda: 'Esc volver' in ui.footer(c.screen()), 'detalle de cambio')
            joined = collect_diff(case, color, before_state['attributes'] & 0x0F)
            flattened = re.sub(r'\s+', '', joined)
            assert digest(ORIGINAL) in flattened and digest(EXPECTED) in flattened
            assert str(last_edit['backup_path']).replace(' ', '') in flattened
            case['checks']['detail_keeps_observed_hashes_and_backup'] = True
            case['checks']['detail_73x21_keeps_cursor_footer_and_pagination'] = True
            c.press(vk=0x1B)
            c.press('\x03')
            c.wait_for(lambda: (folder / 'result').exists(), 'salida y restauracion', seconds=20)
            after_state = c.state()
            assert before_state == after_state, (before_state, after_state)
            assert int((folder / 'result').read_text(encoding='utf-8-sig')) == 0
            case['checks']['console_state_restored'] = True
            assert source.read_bytes() == EXPECTED
            assert digest(verify.read_bytes()) == test_hash
            assert digest(config_path.read_bytes()) == config_hash
            after_test = subprocess.run([args.python, '-B', verify.name], cwd=project,
                                        capture_output=True, text=True, timeout=15)
            case['test_after'] = {'exit_code': after_test.returncode, 'stdout': after_test.stdout,
                                  'stderr': after_test.stderr}
            assert after_test.returncode == 0 and 'VISUAL_SOURCE_PASS' in after_test.stdout
            unexpected = [str(path.relative_to(project)) for path in project.rglob('*') if path.is_file()
                          and not any(part == '.pillcli' or part.startswith('.pill-transaction-') for part in path.relative_to(project).parts)
                          and path.name not in ('calc.py', 'verify.py')]
            assert not unexpected, unexpected
            case['checks']['fixture_test_passes_and_other_files_unchanged'] = True
            case['result'] = 'PASS'
        except BaseException as error:
            case['error'] = repr(error)
            if attached:
                case['failure_screen'] = c.screen()
            raise
        finally:
            if attached and host is not None and host.poll() is None and not (folder / 'result').exists():
                c.press(vk=0x1B)
                c.press('\x03')
                try:
                    c.wait_for(lambda: (folder / 'result').exists(), 'salida tras fallo', seconds=20)
                    case['restored_after_failure'] = before_state == c.state()
                except AssertionError:
                    pass
            if host is not None:
                (folder / 'finish').touch()
                try:
                    host.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    host.terminate()
                    host.wait(timeout=5)
            if attached:
                c.K.FreeConsole()
    print('PASS ' + label, flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--model', required=True)
    parser.add_argument('--python', required=True)
    parser.add_argument('--ollama-url', default='http://127.0.0.1:11434')
    parser.add_argument('--timeout', type=float, default=600)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    if args.evidence.exists():
        raise FileExistsError('No se sobrescribe evidencia existente')
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    before_path = ui.user_path_signature()
    report = {'fecha_utc': datetime.now(timezone.utc).isoformat(), 'role': 'tester independiente',
              'result': 'FAIL', 'binary_sha256': digest(args.binary.read_bytes()),
              'scope': 'Win32 real, modelo local explicito, herramientas y fuentes reales en muestras. Una inferencia activa. Color y sin color, sin clipboard ni registro PATH.',
              'model_metadata': ui.installed_model(args.ollama_url, args.model), 'cases': []}
    try:
        run_case(args, report, True)
        run_case(args, report, False)
        report['result'] = 'PASS'
    except BaseException as error:
        report['error'] = repr(error)
    finally:
        report['user_path_unchanged'] = before_path == ui.user_path_signature()
        report['viewport_trace'] = ui.VIEWPORT_TRACE
        if not report['user_path_unchanged']:
            report['result'] = 'FAIL'
        args.evidence.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({'result': report['result'], 'cases': [case['result'] for case in report['cases']],
                      'error': report.get('error')}, ensure_ascii=True))
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
