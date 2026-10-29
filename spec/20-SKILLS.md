# /skills — listado de skills cargados · 2026-10-29

Aprobado como extensión del tema de skills (plan, fix, refactor, etc.).
Solo lectura; no concede permisos ni activa modelos.

## Alcance
- `/skills` muestra las habilidades cargadas en contexto (plan, fix,
  refactor, report, gepeto, pinokio, opencode) sin modificar nada.
- No se instala ni descarga nada; no cambia `defaults.json` ni PATH.
- Si no hay skills cargadas: mensaje `Sin skills cargados`.

## Diseño
- Lista fija de skills disponibles según `available_skills` del contexto.
- No requiere `nvidia-smi` ni Ollama; es presentación local.

## Comprobación
- `cargo fmt`; `cargo clippy --locked --offline --all-targets -- -D warnings`.
- `cargo test --lib --locked --offline`; `cargo build --release --locked --offline`.
- Tester PASS: `format!` muestra nombres correctos sin errores.
- Reviewer APPROVE.
