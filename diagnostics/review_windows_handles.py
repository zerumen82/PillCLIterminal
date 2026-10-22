"""Prototipos reales de handles Windows usados en la revisión de E3/E4.

Solo crea muestras bajo un directorio temporal propio. No ejecuta modelos.
No es una implementación alternativa de las herramientas del producto.
"""

import ctypes as c
from ctypes import wintypes as w
import datetime
import json
import os
from pathlib import Path
import struct
import sys
import tempfile


class UnicodeString(c.Structure):
    _fields_ = [("length", w.USHORT), ("capacity", w.USHORT), ("buffer", w.LPWSTR)]


class ObjectAttributes(c.Structure):
    _fields_ = [
        ("length", w.ULONG), ("root", w.HANDLE),
        ("name", c.POINTER(UnicodeString)), ("attributes", w.ULONG),
        ("security", w.LPVOID), ("quality", w.LPVOID),
    ]


class IoStatus(c.Structure):
    _fields_ = [("status", c.c_size_t), ("information", c.c_size_t)]


class RenameInfo(c.Structure):
    _fields_ = [("replace", w.DWORD), ("root", w.HANDLE),
                ("length", w.DWORD), ("name", w.WCHAR * 1)]


def main():
    if os.name != "nt":
        raise RuntimeError("Este diagnóstico requiere Windows")
    kernel = c.WinDLL("kernel32", use_last_error=True)
    native = c.WinDLL("ntdll")
    kernel.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, w.LPVOID,
                                  w.DWORD, w.DWORD, w.HANDLE]
    kernel.CreateFileW.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.DeviceIoControl.argtypes = [w.HANDLE, w.DWORD, w.LPVOID, w.DWORD,
                                      w.LPVOID, w.DWORD, c.POINTER(w.DWORD), w.LPVOID]
    kernel.DeviceIoControl.restype = w.BOOL
    kernel.SetFileInformationByHandle.argtypes = [w.HANDLE, c.c_int, w.LPVOID, w.DWORD]
    kernel.SetFileInformationByHandle.restype = w.BOOL
    kernel.ReadFile.argtypes = [w.HANDLE, w.LPVOID, w.DWORD, c.POINTER(w.DWORD), w.LPVOID]
    kernel.ReadFile.restype = w.BOOL
    kernel.SetFilePointerEx.argtypes = [w.HANDLE, c.c_longlong, w.LPVOID, w.DWORD]
    kernel.SetFilePointerEx.restype = w.BOOL
    native.NtCreateFile.argtypes = [c.POINTER(w.HANDLE), w.ULONG,
                                   c.POINTER(ObjectAttributes), c.POINTER(IoStatus),
                                   w.LPVOID, w.ULONG, w.ULONG, w.ULONG,
                                   w.ULONG, w.LPVOID, w.ULONG]
    native.NtCreateFile.restype = c.c_long
    native.NtSetInformationFile.argtypes = [w.HANDLE, c.POINTER(IoStatus),
                                           w.LPVOID, w.ULONG, c.c_int]
    native.NtSetInformationFile.restype = c.c_long
    native.RtlNtStatusToDosError.argtypes = [c.c_long]
    native.RtlNtStatusToDosError.restype = w.ULONG
    handles = []
    reparses = {}
    observations = []

    def open_handle(path, access, share, directory=True):
        handle = kernel.CreateFileW(str(path), access, share, None, 3,
                                    0x02200000 if directory else 0x00200000, None)
        error = c.get_last_error()
        if handle == c.c_void_p(-1).value:
            return None, error
        handles.append(handle)
        return handle, 0

    def reparse(directory, target):
        handle, error = open_handle(directory, 0x100, 7)
        if handle is None:
            return False, error
        substitute = ("\\??\\" + str(target)).encode("utf-16-le")
        display = str(target).encode("utf-16-le")
        body = (struct.pack("<HHHH", 0, len(substitute), len(substitute) + 2,
                            len(display)) + substitute + b"\0\0" + display + b"\0\0")
        data = struct.pack("<IHH", 0xA0000003, len(body), 0) + body
        buffer = c.create_string_buffer(data)
        returned = w.DWORD()
        success = bool(kernel.DeviceIoControl(handle, 0x900A4, buffer, len(data),
                                             None, 0, c.byref(returned), None))
        error = 0 if success else c.get_last_error()
        if success:
            reparses[directory] = handle
        return success, error

    def status_result(status):
        return {"ntstatus": f"0x{status & 0xffffffff:08x}",
                "winerror": native.RtlNtStatusToDosError(status)}

    def create_relative(parent, name):
        buffer = c.create_unicode_buffer(name)
        name_string = UnicodeString(len(name) * 2, len(buffer) * 2,
                                    c.cast(buffer, w.LPWSTR))
        attributes = ObjectAttributes(c.sizeof(ObjectAttributes), parent,
                                      c.pointer(name_string), 0x40, None, None)
        output = w.HANDLE()
        io = IoStatus()
        status = native.NtCreateFile(c.byref(output), 0xC0110000,
                                     c.byref(attributes), c.byref(io), None,
                                     0x80, 1, 2, 0x200060, None, 0)
        if status >= 0:
            handles.append(output.value)
        return status_result(status)

    def rename_relative(source, parent, name, win32=False):
        units = name.encode("utf-16-le")
        buffer = c.create_string_buffer(RenameInfo.name.offset + len(units) + 2)
        information = c.cast(buffer, c.POINTER(RenameInfo))
        information.contents.root = parent
        information.contents.length = len(units)
        c.memmove(c.addressof(buffer) + RenameInfo.name.offset, units, len(units))
        if win32:
            success = bool(kernel.SetFileInformationByHandle(source, 3, buffer, len(buffer)))
            return {"success": success, "winerror": 0 if success else c.get_last_error()}
        io = IoStatus()
        return status_result(native.NtSetInformationFile(source, c.byref(io),
                                                         buffer, len(buffer), 10))

    def check(name, actual, expected):
        passed = actual == expected
        observations.append({"check": name, "actual": actual,
                             "expected": expected, "pass": passed})

    def observed_text(handle):
        assert kernel.SetFilePointerEx(handle, 0, None, 0)
        buffer = c.create_string_buffer(1024)
        length = w.DWORD()
        assert kernel.ReadFile(handle, buffer, len(buffer), c.byref(length), None)
        return buffer.raw[:length.value].decode("utf-8")

    with tempfile.TemporaryDirectory(prefix="pill-review-handles-") as temporary:
        root = Path(temporary).resolve()
        assert root.parent == Path(tempfile.gettempdir()).resolve()
        assert root.name.startswith("pill-review-handles-")
        try:
            outside = root / "outside"
            outside.mkdir()
            weak = root / "attributes-pin"
            weak.mkdir()
            open_handle(weak, 0x80, 3)
            renamed = root / "attributes-pin-renamed"
            weak.rename(renamed)
            check("READ_ATTRIBUTES no impide renombrar un directorio vacío", renamed.is_dir(), True)

            for contents in ("empty", "file", "directory"):
                directory = root / contents
                directory.mkdir()
                if contents == "file":
                    (directory / "child").write_text("contenido", encoding="utf-8")
                elif contents == "directory":
                    (directory / "transaction").mkdir()
                pin, error = open_handle(directory, 0x80000000, 1)
                assert pin is not None, error
                if contents == "empty":
                    writer, write_error = open_handle(directory, 0x40000000, 7)
                    check("GENERIC_READ/shareREAD bloquea GENERIC_WRITE", (writer, write_error), (None, 32))
                success, error = reparse(directory, outside)
                check(f"FSCTL_SET_REPARSE_POINT con {contents}",
                      {"success": success, "winerror": error},
                      {"success": contents == "empty", "winerror": 0 if contents == "empty" else 145})
                if contents == "empty":
                    check("NtCreateFile relativo sobre padre mutado falla sin escape",
                          create_relative(pin, "escaped.txt")["winerror"], 1921)
                    source = root / "mutated-source"
                    source.write_text("base", encoding="utf-8")
                    source_handle, error = open_handle(source, 0x80010000, 1, False)
                    assert source_handle is not None, error
                    check("NtSetInformationFile relativo sobre padre mutado falla",
                          rename_relative(source_handle, pin, "escaped-rename.txt")["winerror"], 1921)
                    check("ninguna publicación en destino externo", list(outside.iterdir()), [])

            normal = root / "native-normal"
            normal.mkdir()
            parent, error = open_handle(normal, 0x80000000, 3)
            assert parent is not None, error
            check("NtCreateFile relativo normal", create_relative(parent, "created.txt")["winerror"], 0)
            check("creación relativa observable", (normal / "created.txt").is_file(), True)
            source = root / "normal-source"
            source.write_text("base", encoding="utf-8")
            source_handle, error = open_handle(source, 0x80010000, 1, False)
            assert source_handle is not None, error
            check("Win32 FileRenameInfo RootDirectory no admitido en este entorno",
                  rename_relative(source_handle, parent, "win32.txt", True)["winerror"], 87)
            check("NtSetInformationFile relativo normal",
                  rename_relative(source_handle, parent, "published.txt")["winerror"], 0)
            check("publicación relativa observable", observed_text(source_handle), "base")
            check("no reemplazar destino existente",
                  rename_relative(source_handle, parent, "created.txt")["winerror"], 183)
            check("base conservada tras colisión", observed_text(source_handle), "base")
        finally:
            # Eliminar solo metadatos junction de nuestras muestras antes del
            # cleanup temporal: ninguna eliminación recursiva atraviesa enlaces.
            for directory, handle in reparses.items():
                data = struct.pack("<IHH", 0xA0000003, 0, 0)
                buffer = c.create_string_buffer(data)
                returned = w.DWORD()
                if not kernel.DeviceIoControl(handle, 0x900AC, buffer, len(data),
                                              None, 0, c.byref(returned), None):
                    raise c.WinError(c.get_last_error())
            for handle in reversed(handles):
                kernel.CloseHandle(handle)

    result = {"utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "scope": "Prototipos del reviewer; no acredita integración Ollama ni producto completo",
              "sources": [
                  "https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew",
                  "https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4aeefef8-92c3-4abc-af7a-a610caf8a165",
                  "https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-zwcreatefile",
              ], "checks": observations,
              "pass": all(check["pass"] for check in observations)}
    destination = Path(sys.argv[1]) if len(sys.argv) > 1 else None
    text = json.dumps(result, ensure_ascii=False, indent=2)
    if destination is not None:
        destination.write_text(text + "\n", encoding="utf-8")
    print(json.dumps({"pass": result["pass"], "checks": len(observations),
                      "evidence": str(destination) if destination else None}))
    return 0 if result["pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
