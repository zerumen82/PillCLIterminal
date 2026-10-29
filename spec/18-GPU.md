# /gpu instantánea · lectura solo VRAM · 2026-10-29

Aprobado como propuesta 8 (solo lectura, sin atribución, sin matar procesos,
sin nuevos permisos, sin instalar modelos ni skills). Solo presentación.

## Alcance

- Nuevo comando `/gpu` que muestra: ruta de `nvidia-smi` (si existe), VRAM
  global usada/libre/libre (MiB), y nota de que no se atribuye a procesos
  individuales (la instantánea es global, no exclusiva).
- No se cambia `defaults.json`, `model`, `PATH`, ni se instala nada.
- Si `nvidia-smi` no existe o falla: mensaje `No disponible` (sin panic).
- El comando no concede permisos nuevos; es solo lectura y nunca modifica
  procesos, GPU o memoria ajena.

## Diseño

- Lectura con `std::process::Command` (`nvidia-smi --query-gpu=memory.used,memory.free,memory.total --format=csv,noheader,nounits`).
- Si el binario no existe (`kind() == NotFound`): mensaje legible `No disponible`.
- Si existe: mostrar línea `VRAM: usado/libre/total MiB` y aviso `Solo lectura · sin atribución individual`.
- No se guarda en diario ni se usa para `release` o `/liberate`.

## Seguridad y límites

- No se ejecuta ningún otro binario; no se lee `PATH` persistente; no se usa
  red. Si `nvidia-smi` tiene argumentos desconocidos, se ignora todo excepto
  el formato especificado.
- El comando `/gpu` no concede ni retira permisos (`/permissions` sigue
  siendo el único para eso).

## Comprobación

- `cargo fmt --all -- --check`; `cargo clippy --locked --offline --all-targets -- -D warnings`.
- `cargo test --lib --locked --offline`; `cargo build --release --locked --offline`.
- Tester independiente PASS: llama `/gpu` con `nvidia-smi` presente (si existe) y sin él; verifica que no modifica permisos ni memoria.
- Reviewer APPROVE sin hallazgos bloqueantes.
- Bitácora actualizada con veredicto PASS o límite declarado (si `nvidia-smi` no está disponible en el entorno, se declara `no disponible`).
