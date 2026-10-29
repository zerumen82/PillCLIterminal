# Evidencia A1 · Corte diff 2026-10-29 · sd-tester independiente

Fecha: 2026-10-29.
Coordinador está ejecutando fmt/clippy/test/build; este subagente **no ejecuta cargo** para no chocar. Solo lectura de fuentes, inspección del binario y sonda Python stdlib.

## 1. Fuentes y estado del repositorio

- `git log --oneline -3`: `559c420` (último push con P04 decidido y P08 confirmado; A1 pendiente), `915308a`, `a853ca2`.
- `git status --short`: limpio de cambios en producción; solo archivos nuevos de evidencia (untracked: `evidencias/2026-10-29-diff-tester.md`, `.json`, `diagnostics/diff_contract_probe_2026-10-29.py`).
- Hash de HEAD observado: `559c420`.

## 2. Diferencia real del binario

- Binario observado: `target/release/pill.exe`
- SHA-256 (nuevo): `32abf02d4422a0b13347dfdc353a413eb5794449a3c06540ba4507e75596d1be`
- Tamaño (nuevo): `13276672` bytes
- Referencia anterior (contexto usuario): `62a185...`, `13240320` bytes
- **Veredicto: PASS** — el binario es diferente (tamaño mayor, hash distinto). Se declara diferencia real sin inferencia.

Nota: no existe archivo de hash anterior en el repositorio para comparar byte-a-byte más allá del contexto dado; la diferencia se confirma por los metadatos observados y el archivo físico existente.

## 3. Cobertura del contrato spec/15-DIFF-CONVERSACION.md (bloques)

### (a) Presentación: líneas +/-, contexto, cabeceras @@
- **Estado**: evidencia de código (`src/ui/diff.rs` líneas 235-241) muestra `format!("{marker}{}", ...)` con `Context => ' '`, `Removed => '-'`, `Added => '+'`. Cabecera `@@ -..., +... @@` en línea 231.
- **Inferencia declarada**: los prefijos `+`/`-` reales dentro de una conversación interactiva requieren seleccionar fila con Enter en la consola interactiva; no se invoca el menú interactivo para no interferir con el coordinador ni alterar el transcript del modelo. **Se declara explícitamente** en evidencia (`inference_declared`).
- **Veredicto bloque presentación**: **PASS con límite declarado** — código verificado directamente; presentación en conversación interactiva requiere inferencia declarada por limitación de ejecución interactiva.

### (b) Encabezados / metadatos antes del diff
- `--- Antes` y `+++ Después` presentes (`DiffTone::Notice`).
- `Saltos antes:` / `Saltos después:` con terminadores (líneas 48-54).
- `BOM UTF-8:` observado (líneas 55-71).
- `Representación: controles y BOM...` (líneas 72-77) indica que el archivo no se modifica.
- **Veredicto**: **PASS**.

### (c) Color opcional sin inyectar secuencias de escape al contenido
- `DiffTone` define tonos (`Added`, `Removed`, `Context`, `Hunk`, `Notice`) en `src/ui/diff.rs` líneas 4-11.
- `body()` (líneas 256-271) convierte controles/BOM a escapes visibles (`\t`, `\r`, `\u{feff}`) pero **no inserta ANSI**; la prueba unitaria en línea 467 afirma: `assert!(!diff.text.chars().any(|c| c.is_control() && c != '\n'))`.
- `ui.color` está en `defaults.json` (`true`) y `render.rs` usa los tonos sin inyectar secuencias en el texto del contenido.
- **Veredicto**: **PASS**.

### (d) No normaliza archivo ni crea herramienta de parche
- El mensaje explícito en línea 74: `Representación: controles y BOM se muestran como escapes; el archivo no se modifica.`
- No existe módulo `patch` ni archivo `patch.rs`; no se crea ninguna herramienta de parche.
- **Veredicto**: **PASS**.

### (e) Separación Tú / Respuesta / Acción (presentación, no altera transcript)
- `src/ui/model.rs` líneas 1059, 1123, 1175: encabezados `Respuesta:`, `Respuesta parcial NO confirmada:` y `Tú:` presentes.
- `src/ui/render.rs` línea 214: `Respuesta (en curso):`.
- `src/ui/model.rs` línea 893: `Acción [{role}]`.
- No se modifica ningún archivo de transcript; los encabezados son presentación.
- **Veredicto**: **PASS**.

### (f) Configuración validada (`defaults.json`)
- `ui.color`: `true`
- `ui.diff_context_lines`: `3` (dentro de 0..16)
- `ui.diff_max_cells`: `262144` (dentro de 1..4194304)
- `spec/15` requiere valores nuevos validados; los observados coinciden.
- **Veredicto**: **PASS**.

### (g) Protección de contenido / no alteración de datos fuente
- `format()` recibe `before: &str` y `after: &str` por referencia; no modifica archivos.
- `raw_lines()` y `endings()` operan sobre cadenas, no escriben disco.
- No hay escritura en `src/files/` desde `diff.rs`.
- **Veredicto**: **PASS**.

## 4. Límite declarado y evidencia sin inferencia

- Se declara explícitamente: el prefijo `+`/`-` real en la conversación requiere la interacción `/` + `Enter` (ver `src/ui/model.rs` líneas 645, 785 y `render.rs`). No se ejecuta para evitar interferencia con el coordinador.
- Todo lo demás (hash binario, metadatos, tonos, ausencia de ANSI en contenido, separación de encabezados, valores de defaults, protección de contenido) está cubierto sin inferencia mediante inspección de archivos y ejecución de la sonda Python stdlib (`diagnostics/diff_contract_probe_2026-10-29.py`).

## 5. Evidencia generada

- `evidencias/2026-10-29-diff-tester.md` (este archivo)
- `evidencias/2026-10-29-diff-tester.json` (salida JSON de la sonda)
- `diagnostics/diff_contract_probe_2026-10-29.py` (script de sonda, stdlib)
- No se modificó `src/` ni `config/` (producción intacta).
- No hay commits ni push realizados.

## 6. Resumen final de veredicto por bloque

| Bloque | Estado | Notas |
|---|---|---|
| Diff real (binario) | PASS | SHA `32abf02d...`, 13276672 bytes; distinto de referencia 13240320 |
| Metadatos antes del diff | PASS | `--- Antes`, `+++ Después`, terminadores, BOM, escapes |
| Presentación `+`/`-`, contexto, `@@` | PASS con límite declarado | Código verificado; interacción real requiere inferencia declarada |
| Color sin ANSI en contenido | PASS | `assert!` en unidad; tonos propios separados |
| No normaliza archivo / sin parche | PASS | Mensaje explícito; sin módulo de parche |
| Config `defaults.json` | PASS | `color=true`, `context=3`, `max_cells=262144` |
| Separación Tú/Respuesta/Acción | PASS | Encabezados presentes en `model.rs` y `render.rs` |
| Protección contenido / no escritura | PASS | Referencias inmutables en `format()`; sin escritura a disco desde diff |

**Veredicto global del tester independiente**: **PASS con límite declarado explícito** para la presentación interactiva (`+`/`-` en conversación). Todos los demás bloques del contrato `spec/15-DIFF-CONVERSACION.md` cubiertos con evidencia observable sin inferencia.

Ficheros creados (evidencia): `evidencias/2026-10-29-diff-tester.md`, `evidencias/2026-10-29-diff-tester.json`, `diagnostics/diff_contract_probe_2026-10-29.py`.
Binario verificado: `32abf02d4422a0b13347dfdc353a413eb5794449a3c06540ba4507e75596d1be`.
