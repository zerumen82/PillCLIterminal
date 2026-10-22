# Guía del repositorio

`pill` 0.1.0 es un agente de programación para la terminal de Windows, escrito en Rust. Se invoca como `pill` desde una consola interactiva ya abierta y habla con Ollama en local. E1, E2 y el corte E3/E4 están entregados con tester PASS y reviewer APPROVE. E5 (cliente MCP stdio y `/liberate`) está implementada y comprobada en local el 2026-10-22; su tester y reviewer independientes siguen pendientes. E6 (cierre de v1) no está hecho. No hay repositorio Git.

## Estructura

- `src/main.rs`, `src/lib.rs`, `src/cli.rs`, `src/launch.rs`: arranque, opciones y exigencia de consola interactiva.
- `src/config/` y `config/defaults.json`: configuración validada. El modelo incluido es `null`.
- `src/ui/`: cápsula, entrada, menú `/` y consola nativa de Windows.
- `src/providers/ollama/`: chat, streaming y errores contra HTTP de loopback.
- `src/files/`: lectura, búsqueda, creación y edición con SHA-256.
- `src/tools/`: herramientas, permisos de rol, comandos concedidos, diario y cliente MCP stdio en `src/tools/mcp.rs`. No existe `src/policy/` ni `src/mcp/`.
- `src/gpu/`: criterio de liberación compartida o exclusiva. No mata procesos.
- `src/agents/`: coordinador, explorador, implementador, tester y reviewer.
- `src/session/`: trabajador de la sesión, aviso de arranque y `/liberate`. La recuperación automática no está implementada.
- `tests/`: `files_real.rs`, `commands_real.rs`, `ollama_live.rs`, `tools_live.rs`, `mcp_stdio.rs` y `gpu_release.rs`.
- `diagnostics/`: sondas de consola y comprobaciones Python de biblioteca estándar.
- `scripts/Install-Pill.ps1`: copia `target/release/pill.exe` a `bin/pill.exe`.
- `assets/`: dibujo Unicode y ASCII de la cápsula.
- `spec/`: especificaciones. Mandan sobre el código.
- `evidencias/`: resultados fechados. No sustituyen una prueba nueva.
- `memory.md`: bitácora.

Mover y eliminar archivos no están implementados. E5 no está cerrada: faltan tester PASS y reviewer APPROVE independientes. El modelo principal sigue sin elegir y el PATH persistente sigue bloqueado.

Antes de añadir funcionalidad, mantener estas especificaciones:

- `spec/00-PRINCIPIOS.md`: principios y definición de listo.
- `spec/01-ARQUITECTURA.md`: arquitectura, módulos y contratos.
- `spec/02-REQUISITOS.md`: requisitos y criterios de aceptación.
- `spec/03-PENDIENTES.md`: deuda técnica y riesgos.
- `spec/04-EJERCITO.md`: roles de agentes y flujo.

`spec/05-E3-E4.md` registra el corte ya ejecutado. Cada decisión, prueba y resultado se anota en `memory.md` con fecha. Las interacciones y la bitácora van en español.

## Comprobación

El proyecto usa Rust 1.94, Clippy y rustfmt. No asumir comandos de otros ecosistemas, como `npm test`.

```powershell
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
cargo build --release --locked --offline
.\scripts\Install-Pill.ps1 -SkipBuild -SessionOnly
python diagnostics/test_console.py
```

Las pruebas de Ollama y de herramientas con modelo son opt-in (`--ignored`) y no cuentan como PASS si no se ejecutan. El detalle está en `README.md`.

## Estilo

Edición Rust 2024. Nombres descriptivos. Los parámetros viven en `config/defaults.json` o en la configuración validada del proyecto. El código de producción no contiene implementaciones vacías, éxitos ficticios ni integraciones simuladas.

## Pruebas

Cerrar una etapa exige formato, Clippy, build, las pruebas aplicables, tester PASS y reviewer APPROVE, con evidencia en `memory.md`. Si un control no existe, se declara no disponible. Un PASS anterior no cubre fuentes que hayan cambiado.

## Commits

No hay historial Git ni una convención de commits aprobada. Hasta entonces, los asuntos serán concisos. No crear commits sin permiso explícito. Una solicitud de revisión describe el problema, el cambio, los requisitos afectados, la evidencia y los límites.

## Reglas de agentes

Leer `C:\Users\INdaHouse\.agents\AGENTS.md`. El usuario aprueba alcance, diseño, dependencias y prioridades. No instalar paquetes, skills ni modelos sin permiso explícito. El cómputo es local primero y cualquier dependencia de red se declara. Pedir permiso antes de borrados, resets o commits. No anunciar E5 como cerrada ni la v1 como terminada: faltan el tester PASS, el reviewer APPROVE y E6. No elegir el modelo principal ni escribir el Registro para sortear P07.
