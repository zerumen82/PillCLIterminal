"""Prueba E1 sobre conhost real oculto, con eventos Win32, sin dependencias externas.

Solo inicia y finaliza sus propios procesos y muestras temporales.
No sustituye Ollama ni capacidades del producto por resultados prefabricados.
"""
import ctypes as c
import argparse
from ctypes import wintypes as w
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
K = c.WinDLL('kernel32', use_last_error=True)
U = c.WinDLL('user32', use_last_error=True)


class Coord(c.Structure):
    _fields_ = [('X', w.SHORT), ('Y', w.SHORT)]


class Rect(c.Structure):
    _fields_ = [('Left', w.SHORT), ('Top', w.SHORT), ('Right', w.SHORT), ('Bottom', w.SHORT)]


class Info(c.Structure):
    _fields_ = [('size', Coord), ('cursor', Coord), ('attributes', w.WORD), ('window', Rect), ('maximum', Coord)]


class Cursor(c.Structure):
    _fields_ = [('size', w.DWORD), ('visible', w.BOOL)]


class Key(c.Structure):
    _fields_ = [('down', w.BOOL), ('repeat', w.WORD), ('vk', w.WORD), ('scan', w.WORD), ('char', w.WCHAR), ('control', w.DWORD)]


class Event(c.Union):
    _fields_ = [('key', Key), ('padding', c.c_byte * 16)]


class Record(c.Structure):
    _fields_ = [('type', w.WORD), ('event', Event)]


K.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, c.c_void_p, w.DWORD, w.DWORD, w.HANDLE]
K.CreateFileW.restype = w.HANDLE
K.CloseHandle.argtypes = [w.HANDLE]
K.AttachConsole.argtypes = [w.DWORD]
K.GetConsoleMode.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]
K.GetConsoleScreenBufferInfo.argtypes = [w.HANDLE, c.POINTER(Info)]
K.GetConsoleCursorInfo.argtypes = [w.HANDLE, c.POINTER(Cursor)]
K.ReadConsoleOutputCharacterW.argtypes = [w.HANDLE, w.LPWSTR, w.DWORD, Coord, c.POINTER(w.DWORD)]
K.WriteConsoleInputW.argtypes = [w.HANDLE, c.POINTER(Record), w.DWORD, c.POINTER(w.DWORD)]
K.GetConsoleWindow.restype = w.HWND
U.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]


def check(ok):
    if not ok:
        raise c.WinError(c.get_last_error())


def handle(name):
    result = K.CreateFileW(name, 0xC0000000, 3, None, 3, 0, None)
    if result == w.HANDLE(-1).value:
        raise c.WinError(c.get_last_error())
    return result


def state():
    i, o = handle('CONIN$'), handle('CONOUT$')
    try:
        im, om, cursor, info = w.DWORD(), w.DWORD(), Cursor(), Info()
        check(K.GetConsoleMode(i, c.byref(im)))
        check(K.GetConsoleMode(o, c.byref(om)))
        check(K.GetConsoleCursorInfo(o, c.byref(cursor)))
        check(K.GetConsoleScreenBufferInfo(o, c.byref(info)))
        return {'input_mode': im.value, 'output_mode': om.value, 'cursor_size': cursor.size,
                'cursor_visible': cursor.visible, 'attributes': info.attributes}
    finally:
        K.CloseHandle(i)
        K.CloseHandle(o)


def screen():
    o = handle('CONOUT$')
    try:
        info, count = Info(), w.DWORD()
        check(K.GetConsoleScreenBufferInfo(o, c.byref(info)))
        width = info.window.Right - info.window.Left + 1
        lines = []
        for row in range(info.window.Top, info.window.Bottom + 1):
            buf = c.create_unicode_buffer(width + 1)
            check(K.ReadConsoleOutputCharacterW(o, buf, width, Coord(info.window.Left, row), c.byref(count)))
            lines.append(buf[:count.value].rstrip())
        return '\n'.join(lines).rstrip()
    finally:
        K.CloseHandle(o)


def dimensions():
    o = handle('CONOUT$')
    try:
        info = Info()
        check(K.GetConsoleScreenBufferInfo(o, c.byref(info)))
        return [info.window.Right - info.window.Left + 1, info.window.Bottom - info.window.Top + 1]
    finally:
        K.CloseHandle(o)


def wait_for(predicate, label, seconds=8):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        result = predicate()
        if result:
            return result
        time.sleep(0.03)
    raise AssertionError('Tiempo agotado: ' + label)


def press(text='', vk=0, control=0):
    i = handle('CONIN$')
    try:
        units = [int.from_bytes(text.encode('utf-16-le')[n:n + 2], 'little') for n in range(0, len(text.encode('utf-16-le')), 2)] if text else [0]
        for unit in units:
            records = (Record * 2)()
            for n, down in enumerate((1, 0)):
                records[n].type = 1
                records[n].event.key = Key(down, 1, vk, 0, chr(unit), control)
            count = w.DWORD()
            check(K.WriteConsoleInputW(i, records, 2, c.byref(count)))
            assert count.value == 2
    finally:
        K.CloseHandle(i)


def run_session(exit_with_control=False):
    captures = {}
    resize = None
    with tempfile.TemporaryDirectory(prefix='pill-e1-') as directory:
        folder = Path(directory)
        # Configuración parcial real, dentro de una muestra descartable.
        project = folder / 'Mi código'
        (project / '.pillcli').mkdir(parents=True)
        preferences = '{"ui":{"poll_ms":100}}' if exit_with_control else '{"ui":{"ascii_logo":true,"logo_color":"cyan"}}'
        (project / '.pillcli/config.json').write_text(preferences, encoding='utf-8')
        startup = subprocess.STARTUPINFO()
        startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
        host = subprocess.Popen([shutil.which('pwsh'), '-NoProfile', '-File', str(ROOT / 'diagnostics/console_host.ps1'),
                                 directory, str(ROOT / 'bin/pill.exe'), str(project)],
                                creationflags=subprocess.CREATE_NEW_CONSOLE, startupinfo=startup)
        try:
            wait_for(lambda: (folder / 'ready').exists(), 'host preparado')
            K.FreeConsole()
            check(K.AttachConsole(host.pid))
            before = state()
            (folder / 'start').touch()
            wait_for(lambda: 'Configuración:' in screen(), 'interfaz iniciada con pill')
            captures['inicio'] = screen()
            assert ('╭─────┬─────╮' if exit_with_control else '.-----+-----.') in captures['inicio']
            assert 'Mi código' in captures['inicio']
            assert 'config.json' in captures['inicio']
            if exit_with_control:
                press('\x03')
            else:
                press('/')
                wait_for(lambda: '> /help ' in screen() and '  /exit ' in screen(), 'menú automático sin Enter')
                captures['barra_sin_enter'] = screen()
                press('e')
                wait_for(lambda: '> /exit ' in screen() and '\n> /help ' not in screen(), 'filtro')
                captures['filtro'] = screen()
                press(vk=0x1B)
                wait_for(lambda: '> /e' in screen() and '/exit ' not in screen(), 'Escape conserva entrada')
                captures['escape'] = screen()
                press(vk=0x08)
                press('h')
                press(vk=0x09)
                wait_for(lambda: '> /help' in screen(), 'Tab completa')
                press(vk=0x0D)
                wait_for(lambda: 'El chat usa herramientas reales' in screen(), 'ayuda real ejecutada')
                captures['ayuda'] = screen()
                press('á🟢z')
                press(vk=0x25)
                press(vk=0x08)
                wait_for(lambda: '> áz' in screen(), 'edición Unicode real')
                captures['unicode'] = screen()
                size_before = dimensions()
                window = K.GetConsoleWindow()
                check(U.SetWindowPos(window, None, 0, 0, 620, 380, 0x0016))
                wait_for(lambda: dimensions()[0] < size_before[0] and '/ comandos' in screen() and all(len(line) < dimensions()[0] for line in screen().splitlines()), 'redimensionado y recorte efectivo')
                resize = {'antes': size_before, 'despues': dimensions()}
                captures['redimensionado'] = screen()
                press(vk=0x24)
                press(vk=0x2E)
                press(vk=0x2E)
                press('/')
                press(vk=0x28)
                press(vk=0x0D)
            wait_for(lambda: (folder / 'result').exists(), 'salida del programa')
            code = int((folder / 'result').read_text(encoding='utf-8-sig').strip())
            assert code == 0, code
            after = state()
            assert before == after, (before, after)
            return {'salida': 'Ctrl+C' if exit_with_control else '/exit con flecha y Enter',
                    'codigo': code, 'estado_antes': before, 'estado_despues': after, 'redimensionado': resize, 'capturas': captures, 'resultado': 'PASS'}
        finally:
            (folder / 'finish').touch()
            if host.poll() is None and not (folder / 'result').exists():
                press('\x03')
            try:
                host.wait(timeout=3)
            except subprocess.TimeoutExpired:
                # Únicamente el host creado por esta prueba.
                host.terminate()
                host.wait(timeout=3)
            K.FreeConsole()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--evidence', default=str(ROOT / 'evidencias/2026-10-20-e2-regresion-consola.json'))
    args = parser.parse_args()
    sessions = [run_session(), run_session(exit_with_control=True)]
    binary = ROOT / 'bin/pill.exe'
    report = {'fecha_utc': datetime.now(timezone.utc).isoformat(), 'tipo': 'Regresión E1 sobre implementación actual; consola Windows real; sin inferencia',
              'binario': str(binary), 'sha256_binario': hashlib.sha256(binary.read_bytes()).hexdigest(), 'sesiones': sessions, 'resultado': 'PASS'}
    output = Path(args.evidence)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print('PASS: menú sin Enter, filtro, Esc, Tab, ayuda, Unicode, resize, /exit y Ctrl+C; modos/cursor/atributos restaurados.')
