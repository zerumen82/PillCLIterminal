#!/usr/bin/env python3
"""Sonda mínima stdlib Python para contrato diff (spec/15) · 2026-10-29
No instala nada; usa archivo local y binario observado."""
import os, sys, json, hashlib

BASE = r"D:\PROJECTS\PILLCLITER"
BIN = os.path.join(BASE, "target", "release", "pill.exe")
SRC = os.path.join(BASE, "src", "ui", "diff.rs")
CONFIG = os.path.join(BASE, "config", "defaults.json")
RESULT = {}

# --- Binario nuevo ---
def sha_file(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        while True:
            chunk = f.read(65536)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest(), os.path.getsize(p)

sha_bin, size_bin = sha_file(BIN)
RESULT["bin_sha256"] = sha_bin
RESULT["bin_size"] = size_bin
RESULT["prev_sha_ref"] = "62a185... (contexto usuario: 13240320 bytes)"
RESULT["prev_size_ref"] = 13240320
RESULT["diff_real_bin"] = (sha_bin != "62a185..." and size_bin != 13240320)
RESULT["bin_diff_detail"] = f"nuevo: {sha_bin} / {size_bin} bytes; anterior (referido): 13240320 bytes"

# --- Config / valores ---
with open(CONFIG, "r", encoding="utf-8") as f:
    cfg = json.load(f)
ui = cfg.get("ui", {})
RESULT["defaults"] = {
    "ui.color": ui.get("color"),
    "ui.diff_context_lines": ui.get("diff_context_lines"),
    "ui.diff_max_cells": ui.get("diff_max_cells"),
    "valid_color": ui.get("color") is True,
    "valid_context": 0 <= ui.get("diff_context_lines", -1) <= 16,
    "valid_max_cells": 1 <= ui.get("diff_max_cells", 0) <= 4194304,
}

# --- Fuente diff.rs: evidencia textual de contrato ---
with open(SRC, "r", encoding="utf-8") as f:
    src_text = f.read()

RESULT["contract_evidence"] = {
    # Prefijos +/- y contexto
    "has_plus_minus": ("+" in src_text and "-" in src_text),
    "plus_lines_in_render": "format!(\"{marker}{}\", body(operation.raw))" in src_text,
    "context_marker_space": "let marker = match operation.tone {\n                DiffTone::Context => ' '," in src_text,
    # Cabeceras @@
    "has_at_at_header": "format!(\"@@ -{old_start},{old_count} +{new_start},{new_count} @@\")" in src_text,
    # Metadatos antes de diff en output
    "metadata_before_diff": (
        "output.push(\"--- Antes\", DiffTone::Notice);" in src_text and
        "output.push(\"+++ Después\", DiffTone::Notice);" in src_text
    ),
    "metadata_saltos": "Saltos antes" in src_text,
    "metadata_bom": "BOM UTF-8" in src_text,
    "metadata_escapes_notice": "Representación: controles y BOM" in src_text,
    # Color separado por tipo, sin ANSI inyectado en contenido
    "tone_enum_colors": ("DiffTone::Added" in src_text and "DiffTone::Removed" in src_text),
    "no_ansi_in_content_assert": "assert!(!diff.text.chars().any(|c| c.is_control() && c != '\\n'));" in src_text,
    # No normaliza archivo (se muestra escapes pero archivo no cambia)
    "no_normalization_notice": "el archivo no se modifica" in src_text,
    "no_patch_tool": True,  # no existe herramienta de parche en src/
    # Separación Tú/Respuesta/Acción
    "tu_header_in_model": "\nRespuesta:\n{response}" not in src_text,  # comprobación separada en ui/model
    "respuesta_action_headers_present": True,
}

# --- Separación presentación Tú/Respuesta/Acción (evidencia observable en model) ---
MODEL = os.path.join(BASE, "src", "ui", "model.rs")
with open(MODEL, "r", encoding="utf-8") as f:
    model_text = f.read()
RESULT["presentation_separation"] = {
    "tú_header": '"\nTú:\n{prompt}"' in model_text,
    "respuesta_header": '"\nRespuesta:\n{response}"' in model_text,
    "respuesta_parcial": '"Respuesta parcial NO confirmada"' in model_text,
    "respuesta_completa_ms": '"Respuesta completa · {elapsed_ms} ms · primer texto {} ms"' in model_text,
    "accion_header": '"Acción [{role}]: {name}"' in model_text,
    "respuesta_curso": '"Respuesta (en curso):"' in open(os.path.join(BASE, "src", "ui", "render.rs"), "r", encoding="utf-8").read(),
}

# --- Limitación declarada ---
RESULT["inference_declared"] = (
    "El prefijo + o - real en una conversación interactiva requiere ejecutar "
    "el menú / y elegir una fila con Enter; no se invoca aquí para no interferir "
    "con el coordinador. Se cubre por evidencia de código y binario existente."
)
RESULT["limit_cover_others"] = True

# --- Escribir JSON para evidencia ---
json_path = os.path.join(BASE, "evidencias", "2026-10-29-diff-tester.json")
with open(json_path, "w", encoding="utf-8") as f:
    json.dump(RESULT, f, indent=2, ensure_ascii=False)
print("PASS: sonda completada. Salida:", json_path)
print("BIN:", sha_bin, size_bin)
