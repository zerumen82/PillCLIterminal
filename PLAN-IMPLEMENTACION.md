# Plan de implementación · pill

Estado vigente 2026-10-28: mejora integral U01-U15 verificada con tester PASS,
reviewer APPROVE e instalacion real de pill en User PATH (P07 cerrado).
E6/v1 conserva P04/P08. Los estados de fechas anteriores son historicos;
fuentes y evidencia actuales en [memory.md](memory.md).


Fecha: 2026-10-20. Estado: aprobado por el usuario junto con las dependencias; base E1 e integración E2 verificadas, tester PASS y reviewer APPROVE. Precisión aprobada: invocación `pill`, exclusivamente desde consola interactiva existente.

Seguimiento E3/E4: implementadas herramientas de archivos y comandos concedidos y flujo de cinco roles. Formato/lint/48 pruebas/build offline PASS, reviewer APPROVE y tester de consola PASS (26/26 checks) con perfil explícito de razonamiento. Los fallos observados motivaron feedback/reintentos acotados y traspasos de hechos reales; se conservan sus informes. Contratos, evidencia y límites en [ejecución E3/E4](spec/05-E3-E4.md) y [requisitos actuales](spec/02-REQUISITOS.md#continuación-e3e4--2026-10-20). R05 mantiene lecturas paralelas pendientes; mover/eliminar y recuperación automática siguen sin entregar. Ese corte no cubre el código de E5.

Seguimiento E5 al 2026-10-22: cliente MCP en `src/tools/mcp.rs` y liberación en `src/gpu/` y `src/session/`. Contrato en [spec/06-E5.md](spec/06-E5.md). Formato, Clippy con warnings denegados, 60 pruebas locales y build release offline PASS; cinco pruebas opt-in de Ollama siguen ignoradas. Release y `bin/pill.exe` idénticos, SHA-256 `8FEA4F07FDC6F7532EB287EADDAD940A0AA6C6C25EF68E6F8E1E922B5490E9F5`. La sonda de consola E1 pasó sobre ese binario. No hay tester PASS ni reviewer APPROVE de E5, así que la etapa no está cerrada y no se declara v1. Este párrafo prevalece sobre cualquier frase anterior que deje MCP o `/liberate` sin implementar.

Seguimiento 2026-10-27: el usuario aprobó completar pendientes y el diseño de archivos, concurrencia y recuperación. Contratos en `spec/07-R09.md`, `spec/08-RECUPERACION.md` y `spec/09-CIERRE-20261027.md`; implementación verificada con 119 pruebas locales, siete integraciones opt-in, tester PASS y reviewer APPROVE del corte E5 y ampliaciones. E6 sigue pendiente por P04/P07/P08. No se incorporan dependencias ni modelos nuevos.

## 1. Contexto y decisiones

Construir el agente de programación local en terminal para Windows. El usuario ha aprobado Rust, Ollama inicialmente, multiagentes y MCP `stdio` en v1, cápsula propia, menú `/`, `/liberate`, lectura/escritura sin confirmación por archivo y pruebas/build en el proyecto. Instalaciones y acciones destructivas requieren confirmación. La búsqueda web se incorporará después del ciclo local.

Las variantes Huihui 4B/4B-v2/2B están descargadas. En E2, los tres candidatos pasaron una muestra real de chat y streaming con contexto efectivo 4096; datos y límites en [la comparación](evidencias/2026-10-20-e2-comparacion.md). La prueba no ejecuta el código generado ni elige un modelo principal. No es necesario descargar modelos nuevos.

Especificaciones: [principios](spec/00-PRINCIPIOS.md), [arquitectura](spec/01-ARQUITECTURA.md), [requisitos](spec/02-REQUISITOS.md), [pendientes](spec/03-PENDIENTES.md), [roles](spec/04-EJERCITO.md).

## 2. Cumplimiento y propuestas para aprobar

Se respetan [AGENTS.md](AGENTS.md), las reglas globales y la skill `plan`: especificar primero y aprobar antes de implementar o instalar dependencias. Parámetros centralizados, operaciones reales, cómputo local y bitácora con evidencia. Desde el 2026-10-22, `AGENTS.md` describe el repositorio real. Hasta esa fecha se había conservado como andamiaje vacío por una instrucción anterior.

El usuario ha aprobado este plan y sus dependencias, con los detalles siguientes:

- Raíz inicial: proyecto activo. Lectura/creación/edición completas dentro de él; nuevas raíces solo por decisión explícita. Borrados y acciones destructivas conservan confirmación concreta.
- Configuración JSON; una inferencia activa, hasta dos exploraciones nativas independientes y una aplicación de cambios a la vez. Todos esos límites configurables.
- Configuración del repositorio sin capacidad de autoaprobar permisos; concesiones por proyecto separadas en `%LOCALAPPDATA%\PILLCLI\trust.json`, modificables solo mediante interacción explícita del usuario. Cambios en definiciones de comandos, scripts o servidores requieren renovar la concesión.
- Interfaz nativa Windows con `windows-sys`, elegida expresamente por el usuario como alternativa a Ratatui/Crossterm. Opciones mediante biblioteca estándar en lugar de Clap. Menú filtrable con flechas/Enter/Esc y comandos iniciales `/help`, `/liberate`, `/exit`, disponibles cuando su implementación real esté terminada. Cápsula dibujada en terminal con versión ASCII.
- Servidor Ollama compartido por defecto: liberar solo recursos cuya descarga pueda autorizarse sin afectar clientes ajenos. No adoptar automáticamente exclusividad ni descargar recursos de ComfyUI.
- Autorización automática de las acciones Rust de formato/lint/test/build indicadas abajo; otros scripts necesitan registro explícito. Ningún nombre de prueba convierte un comando arbitrario en aprobado.

## 3. Dependencias propuestas y pasos

Rust/Cargo 1.94.0, Clippy y rustfmt están presentes. El enlace MSVC y la ejecución de E1 están verificados. No se autoriza instalar herramientas del sistema ni actualizar Rust.

Versiones directas iniciales propuestas; se fijarán en `Cargo.toml` y las transitivas en `Cargo.lock` tras autorización. Versiones seleccionadas por documentación publicada y, cuando existe, caché local; compatibilidad completa del árbol y licencias transitivas todavía no verificadas. Si exigen otro toolchain o paquetes directos distintos, detener ese cambio y consultar.

| Paquete / versión inicial | Uso y alcance |
|---|---|
| `tokio 1.52.3` | Tareas, red/procesos asíncronos y cancelación coordinada |
| `reqwest 0.13.3` | Cliente HTTP de Ollama, JSON y streaming; sin TLS en la primera conexión HTTP local |
| `futures-util 0.3.32` | Consumo de streams y composición asíncrona |
| `serde 1.0.228`, `serde_json 1.0.149` | Tipos, configuración, mensajes y diario JSON |
| `rmcp 3.6.0` | SDK MCP oficial; cliente y transporte de procesos hijos, sin HTTP ni instalación de servidores |
| `jsonschema 0.59.0` | Validación de argumentos MCP; resolución externa de referencias deshabilitada |
| `sha2 0.10.9` | Identificar bases y contenido comprobado de archivos |
| `tempfile 3.27.0` | Archivos temporales y proyectos de pruebas descartables |
| `thiserror 2.0.18` | Errores tipados y diagnósticos |
| `windows-sys 0.61.2` | Funciones Windows necesarias para procesos y operaciones de archivos; solo en Windows |

El plan original aprobó 14 paquetes directos. Tras retirar Ratatui/Crossterm/Clap por elección del usuario, quedan los 11 de esta tabla, contando `serde` y `serde_json` por separado. E1 usó cuatro: serde, serde_json, thiserror y windows-sys. E2 añade Tokio, Reqwest y Futures: siete directas y 81 paquetes externos en el árbol Windows; resolución y compilación offline, sin descargas. La autorización incluye las transitivas necesarias para esas versiones, no paquetes opcionales ajenos ni cambios de versión mayor. Reducir features: `rmcp` solo cliente/child-process; `reqwest` JSON/stream con proxy deshabilitado para loopback; `jsonschema` sin resolución HTTP/archivos externos. No usar un framework adicional de agentes.

Fuentes principales: [RMCP](https://docs.rs/rmcp/3.6.0/rmcp/), [JSON Schema](https://docs.rs/jsonschema/0.59.0/jsonschema/), [Reqwest](https://docs.rs/reqwest/0.13.3/reqwest/). E1 se resolvió y compiló desde la caché local; no se descargaron paquetes nuevos.

### Etapas de implementación propuestas

Seguimiento histórico al 2026-10-22 (la continuación vigente está en la cabecera): E1 y E2 cerradas con tester PASS y reviewer APPROVE. E2 cubre tres modelos, errores reales, cancelación/reintento y consola con entrada durante la generación. E3 y E4 están entregadas en el alcance de la cabecera: lectura, creación, edición, comandos concedidos y cinco roles. La política vive en `src/tools/`; no existe `src/policy/` ni `src/mcp/`. El cliente MCP está en `src/tools/mcp.rs`. Las pruebas reales están en `tests/files_real.rs`, `tests/commands_real.rs`, `tests/tools_live.rs`, `tests/mcp_stdio.rs` y `tests/gpu_release.rs`. No existen `tests/files/`, `tests/agents/`, `tests/fixtures/` ni `tests/gpu/`. Mover y eliminar (R09) siguen pendientes. E5 está implementada en local y pendiente de cierre independiente. E6 pendiente. P07 (PATH persistente) y P04 (modelo principal) siguen abiertos. La herramienta de tareas no está disponible en esta sesión; se registra el seguimiento aquí y en `memory.md`.

El usuario eligió consola nativa y desbloqueó E1. El comando `pill` se ha verificado desde `bin/` mediante PATH de sesión. La inscripción de PATH persistente está preparada en `scripts/Install-Pill.ps1`, pero el entorno rechazó el acceso al Registro; requiere ejecución en la consola habitual del usuario. No se instaló una aplicación gráfica.

| Etapa | Estado | Archivos reales o todavía previstos | Resultado verificable |
|---|---|---|---|
| E1 · Base y terminal | Entregada | `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `src/lib.rs`, `src/cli.rs`, `src/launch.rs`, `src/config/`, `src/ui/`, `config/defaults.json`, `README.md` | Ejecutable real, cápsula, entrada y menú automático; restauración y cancelación |
| E2 · Ollama | Entregada | `src/providers/ollama/`, `src/session/`, `tests/ollama_live.rs` | Elegir modelo, generar y detectar errores reales; comparar candidatos con memoria y tiempos |
| E3 · Archivos | Entregada en lectura, creación y edición | `src/files/`, `src/tools/`, `tests/files_real.rs` | Leer, buscar, crear, editar y releer; conflictos de un archivo probados. Mover y eliminar siguen pendientes. No hay `src/policy/` |
| E4 · Agentes y comandos | Entregada en la muestra de cinco roles | `src/agents/`, `src/tools/commands.rs`, `tests/commands_real.rs`, `tests/tools_live.rs` | Delegación real con historiales propios y corrección de una muestra con comprobaciones. No hay `tests/agents/` ni `tests/fixtures/` |
| E5 · MCP y GPU | Implementada en local; cierre independiente pendiente | `src/tools/mcp.rs`, `src/gpu/`, `tests/mcp_stdio.rs`, `tests/gpu_release.rs`, `diagnostics/mcp_fixture.py` | Cliente stdio real con concesión; arranque y `/liberate` comparten criterio. El stub local conserva lo ajeno y descarga solo lo seleccionado. No sustituye tester ni reviewer |
| E6 · Cierre v1 | Pendiente | `README.md`, `spec/02-REQUISITOS.md`, `spec/03-PENDIENTES.md`, `memory.md`, `evidencias/` | Criterios R01–R15 verificados, informe de límites y revisión independiente |

En E1 no se mostrarán herramientas, agentes o limpieza como disponibles si aún no lo están. El menú solo anuncia capacidades implementadas: `/liberate` se registra como ejecutable en E5. La v1 completa no se entrega antes de implementarlo y verificarlo. Cada etapa funciona dentro de su alcance y no aparenta la v1 completa mediante stubs.

## 4. Riesgos y límites

ComfyUI y otros procesos usan la GPU; conservarlos. No afirmar que el modelo cabe íntegramente en VRAM sin medirlo. Los modelos pueden generar código incorrecto o llamadas inválidas. Borrado recursivo, web, MCP remoto y sandbox quedan fuera de esta primera entrega. Las instalaciones de paquetes Rust se hacen después de aprobación; ningún servidor MCP externo ni nuevo modelo está incluido.

Publicación de archivos frente a carreras externas y recuperación tras cierres requieren pruebas específicas antes de afirmar garantías. Cambios de varios archivos pueden ser parciales. La política del CLI no convierte scripts ni servidores externos en procesos aislados.

## 5. Impacto

Al redactar el plan no había código. Desde E1 el repositorio contiene la aplicación descrita en el seguimiento de etapas. El diseño usa un proceso de CLI y el servidor Ollama existente; permite probar la interacción real sin imponer varios modelos residentes. Etapas y límites evitan mezclar éxito documental con funcionamiento. Las dependencias implican tiempo de compilación y mantenimiento; se reducirán features y se documentará cualquier deuda en `spec/03-PENDIENTES.md`.

## 6. Verificación propuesta

Después de aprobar dependencias: resolver versiones, revisar licencias/compatibilidad, generar lockfile y compilar sin instalar toolchains adicionales. Después de cada etapa, ejecutar únicamente los controles aplicables; al cierre:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

Las pruebas reales de Ollama se ejecutarán expresamente sobre el modelo seleccionado y se registrará cualquier integración omitida. Las muestras, temporales y procesos de pruebas pertenecen a la prueba y podrán limpiarse dentro de ese alcance; esto no autoriza borrar archivos del usuario ni detener otros procesos.

Verificar terminal real y menú, ciclo modelo/herramienta, archivos en disco, conflicto entre agentes, prueba de corrección, servidor MCP local y `/liberate` con recurso compartido y exclusivo autorizado. Registrar versión de archivos, digest de modelo, configuración, resultados y evidencia. Se exige tester PASS y reviewer APPROVE sobre la versión final. E1 ya dispone de lint, build y pruebas de consola ejecutables; las demás integraciones se verificarán en sus etapas.
