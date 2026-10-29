# Menú agrupado e indicador de posición · 2026-10-29

Aprobado como entrega pendiente de `spec/15-DIFF-CONVERSACION.md` (fue declarada
«para otra entrega»). Ningún cambio a modelo, permisos, PATH, dependencias,
diff, color ni transcript del modelo. Solo presentación de UI.

## Alcance aprobado (A)

- **Agrupación del menú `/`**: los comandos (`/help`, `/exit`, `/model`, `/permissions`, `/task`, `/chat`, `/journal`, `/liberate`, `/status`, `/ask`, `/edit`, `/changes`, `/recover`, `/undo`, `/profile`, `/archive`, `/copy`) se presentan agrupados por categoría visible (Modos, Configuración, Recuperación, Salida) sin alterar los nombres ni sus acciones.
- **Indicador de posición en detalle**: cuando se abre un detalle (`/profile`, `/permissions`, `/changes`, `/recover`), el pie o cabecera muestra `posición X / total` y `Ctrl+Enter` confirma aplicación, `Enter` muestra explicación (ya existente; solo se hace visible el número de posición para no perder contexto).
- **Conservación**: la caché Arc de líneas envueltas (`spec/14`) y la selección visible del cortamiento se mantienen; no se regresa a copiar todo el historial por frame.

## Diseño

- Grupos por orden: Modos (`/ask`, `/edit`, `/chat`, `/task`), Config/Estado (`/model`, `/status`, `/permissions`, `/journal`, `/profile`), Recuperación (`/changes`, `/recover`, `/undo`, `/archive`), Salida (`/exit`, `/copy`, `/liberate`).
- Posición en detalle: `Detalle: X / N · Enter = explicación · Ctrl+Enter = confirmar` (aparece solo si hay detalle abierto; sin detalle no se muestra, para no ocupar espacio en la cabecera compacta).

## Configuración

Sin nuevos parámetros en `defaults.json`; usa `ui.compact_header` existente. Si `compact_header=true`, el indicador de posición se omite de la cabecera y aparece solo en el pie del detalle.

## Comprobación

- `cargo fmt` PASS; `cargo clippy --locked --offline --all-targets -- -D warnings`; `cargo test --lib --locked --offline`; `cargo build --release --locked --offline`.
- Tester independiente PASS sobre menú agrupado y posición visible.
- Reviewer APPROVE sin hallazgos bloqueantes.
- Bitácora (`memory.md`) actualizada.
