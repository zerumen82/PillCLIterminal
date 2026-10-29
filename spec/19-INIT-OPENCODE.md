# /init y revisión de configs OpenCode · 2026-10-29

Aprobado como extensión de ejecución/instalación (P07). No amplía permisos,
no instala modelos ni skills, no cambia `model`. Solo validación y reporte
de configuraciones locales.

## Alcance aprobado

- **`/init`**: valida `config/defaults.json` (rangos, campos conocidos, `model`
  no vacío vacío; conserva `null` como opción válida si se indica); confirma
  que `bin/` existe y que PATH persistente (`HKCU:\Environment`) incluye el
  directorio; reporta `opencode` agent settings (lee `C:\Users\INdaHouse\.agents\AGENTS.md`
  si existe, declara si está vacío/faltante, sin modificarlo); muestra resumen
  `PASS`/`WARN`/`FAIL` por bloque.
- **Revisión opencode**: lectura de `AGENTS.md` global/local, verificación de
  que existe y tiene contenido mínimo (título + flujo); sin escribir, sin
  borrar, sin tocar modelos ni PATH.
- **Sin acciones con efectos**: `Enter` muestra detalle; `Ctrl+Enter`
  confirma solo si hay error bloqueante (ej. `defaults.json` inviable); nunca
  concede permisos ni modifica registro sin paso explícito.

## Diseño

- `/init` invoca: `Config::load` + verificación de archivo físico `bin/` +
  lectura `AGENTS.md`; mensaje conciso `PASS: config + PATH + opencode` o
  `WARN: <bloque>` con detalle por línea.
- Si `AGENTS.md` falta: `WARN: opencode agent settings no encontrados`.
- Si `defaults.json` tiene campo desconocido o rango inválido: `FAIL` con
  nombre del campo.

## Configuración

Sin nuevos parámetros. Usa `config/defaults.json` existente.

## Comprobación

- `cargo fmt`; `cargo clippy --locked --offline --all-targets -- -D warnings`.
- `cargo test --lib --locked --offline`; `cargo build --release --locked --offline`.
- Tester PASS: `/init` con config válida y con `defaults.json` alterado
  artificialmente (campo desconocido) muestra `FAIL` sin afectar producción.
- Reviewer APPROVE sin hallazgos bloqueantes.
