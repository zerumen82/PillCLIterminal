# Estado UI: plan / actuar / modo · 2026-10-29

Aprobado como extensión de presentación (`UI`). No cambia `defaults.json`, `model`,
`PATH`, ni concede permisos. Solo lectura del contexto actual.

## Alcance
- **Estado `plan`**: si `plan` está cargado (skill disponible), mostrar línea
  visible en cabecera o pie: `Plan: cargado` (o `Plan: no cargado`).
- **Estado `actuar` / modo agente**: si `task_mode` o `agent` activo, mostrar
  `Estado: Tarea · roles: coordinador, explorador, implementador, tester, reviewer`
  (o `Estado: Chat` / `Estado: Preguntar` según modo actual).
- **Cualquier otro estado**: el usuario puede añadir líneas de estado adicionales
  mediante la configuración (`defaults.json` no se modifica; se usa el contexto
  existente `App`).

## Diseño
- En `render.rs`: si `compact_header` es `true`, la línea de contexto (`Reserva:`) se amplía con el estado actual (`Plan` + `Estado`).
- Sin nuevos comandos (`/plan` o `/actuar` no se crean); es solo presentación.
- No se modifica `src/config/mod.rs`.

## Comprobación
- `fmt`/`clippy`/`test` PASS; `build` PASS; evidencia breve.
