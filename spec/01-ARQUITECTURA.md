# Arquitectura y contratos

Estado: aprobado el 2026-10-20 mediante [el plan](../PLAN-IMPLEMENTACION.md), con invocación `pill` solo desde consola. Base E1/E2, herramientas nativas y muestra de corrección con cinco roles E3/E4 verificadas con tester PASS y reviewer APPROVE. El 2026-10-22 E5 quedó implementada y comprobada en local; su tester y reviewer independientes siguen pendientes. Lecturas paralelas, mover/eliminar, recuperación automática y E6 siguen sin entregar.

Actualización aprobada: el usuario eligió consola nativa de Windows ante el bloqueo de dependencias. E1 emplea `windows-sys` y la biblioteca estándar para pantalla, teclado y opciones; se retiraron Ratatui, Crossterm y Clap. El comando público sigue siendo `pill`, con el binario de consola como implementación interna, sin lanzador gráfico.

## Procesos y módulos

Un ejecutable `pill.exe` mantiene interfaz, coordinación y ejecutor. El control de entrada implementado exige stdin/stdout de terminal y, en Windows, más de un proceso asociado a consola: el caso probado de consola propia con un único proceso se rechaza. Este criterio no identifica al invocador ni la antigüedad de la consola. Ayuda y versión son salidas informativas, no inicio de interfaz. Ollama es un servidor separado ya instalado. Los agentes son tareas con conversación propia; no requieren varios procesos ni varias copias de un modelo. Un servidor MCP concedido es un proceso hijo local; sin concesión no se arranca.

| Ruta | Estado | Responsabilidad |
|---|---|---|
| `src/main.rs`, `src/lib.rs`, `src/cli.rs`, `src/launch.rs` | Presente | Arranque, opciones, composición y consola interactiva |
| `src/config/`, `config/defaults.json` | Presente | Configuración, origen de cada valor y validación |
| `src/ui/` | Presente | Cápsula, entrada, menú de comandos, progreso y cancelación |
| `src/providers/ollama/` | Presente | Metadatos, chat, streaming y errores de la API |
| `src/agents/` | Presente | Coordinador, explorador, implementador, tester y reviewer |
| `src/tools/` | Presente | Registro, autorización, comandos concedidos, diario y cliente MCP stdio en `src/tools/mcp.rs`. No existe `src/policy/` ni `src/mcp/` |
| `src/files/` | Presente | Versiones de base, creación, edición y comprobación en disco |
| `src/session/` | Presente | Operación activa, aviso de arranque y `/liberate`. La recuperación automática sigue pendiente |
| `src/gpu/` | Presente | Plan de liberación y medición NVIDIA. La descarga HTTP vive en el proveedor Ollama |
| `tests/` | Presente | `files_real.rs`, `commands_real.rs`, `ollama_live.rs`, `tools_live.rs`, `mcp_stdio.rs`, `gpu_release.rs`. No hay `tests/fixtures/` ni `tests/gpu/` |
| `evidencias/` | Presente | Resultados fechados de verificaciones |

E1 tiene `src/`, `config/`, `assets/`, `scripts/` y pruebas de consola en `diagnostics/`. E2 añadió el proveedor Ollama, la sesión y `tests/ollama_live.rs`. E3/E4 incorpora `src/files/`, `src/tools/`, `src/agents/` y las pruebas reales de archivos, comandos y modelos. E5 añade `src/tools/mcp.rs`, `src/gpu/` y las pruebas `tests/mcp_stdio.rs` y `tests/gpu_release.rs`. El servidor MCP de prueba es el proceso local `diagnostics/mcp_fixture.py`; no sustituye un servidor de producción elegido por el usuario.

## Configuración propuesta

JSON: valores incluidos en `config/defaults.json`, configuración local `.pillcli/config.json` y opciones explícitas de línea de comandos, en ese orden de precedencia. Un esquema inválido produce diagnóstico de campo y no se ignora. Mostrar qué archivo y opciones gobiernan la sesión.

La precedencia de valores no concede permisos. Cargar `.pillcli/config.json`, otro archivo indicado por `--config` o la configuración MCP de un repositorio solo propone raíces adicionales, comandos, servidores y modo exclusivo; no los activa sin consentimiento explícito del usuario. Las opciones ordinarias de modelo/interfaz pueden aplicarse dentro del alcance ya autorizado. La raíz seleccionada expresamente mediante `--project` es el alcance inicial propuesto; una raíz extra escrita por el repositorio no amplía ese alcance.

Propuesta: conservar concesiones por proyecto en `%LOCALAPPDATA%\PILLCLI\trust.json`, separadas de la configuración del repositorio y fuera del alcance de edición de herramientas del modelo. Solo una interacción explícita del usuario puede registrar o revocar concesiones; no pueden crearlas agentes ni heredarlas del texto de un archivo. Vincularlas a raíz canónica y definición aprobada: programa, argumentos, directorio, script/servidor y modo de acceso. Si cambia esa definición o el archivo de script/servidor aprobado, invalidar su concesión y mostrar la nueva propuesta. El registro del ejecutor y este almacén permanecen protegidos incluso si el usuario amplía otras raíces de trabajo.

Los modelos se descubren por API y se seleccionan mediante interfaz, configuración u opción `--model`; no fijar nombres Huihui en el código. En el primer arranque, seleccionar explícitamente entre modelos disponibles. Sin cambios automáticos de modelo ni descargas automáticas.

### Contrato E2 implementado

Este apartado conserva el contrato de la entrega histórica E2. El ciclo de herramientas vigente se describe en el contrato E3/E4 más abajo.

Un hilo trabajador mantiene un runtime Tokio y una operación local activa. La consola intercambia acciones y eventos mediante colas acotadas; se reserva espacio para el evento terminal y se cancela mediante una generación observable. Salir cancela y une el trabajador. La entrada y el redimensionado no esperan a la red.

La selección consulta `/api/tags` y `/api/show`, valida contexto y capacidades y vuelve a comprobar el digest. Cada chat comprueba el digest antes y después de procesar NDJSON con límites de bytes/línea. Solo un final confirmado añade petición y respuesta al historial. Selección confirmada inicia otra conversación; fallos y cancelaciones conservan las respuestas parciales como tales en pantalla y no las envían en peticiones posteriores. E2 mantiene el chat en memoria, sin recuperación persistente.

El proveedor permite exclusivamente HTTP loopback; fija `localhost` a loopback IPv4, deshabilita proxies y rechaza redirecciones. No envía `keep_alive` ni solicita descarga/liberación de modelos. Las herramientas solicitadas por el modelo se rechazan como no disponibles. Las métricas reales se conservan en el resultado y evento final; la UI muestra recuentos y carga si existen. `/api/ps` y los recursos se registran en diagnósticos externos; la UI no presenta contexto solicitado como efectivo.

Ctrl+Enter envía peticiones y Enter añade línea; los comandos del menú conservan Enter. Esc cancela operaciones o cierra el menú. El historial se acota mediante una estimación conservadora de bytes y envoltorio; no se anuncia un contador exacto de tokens. Los parámetros se concentran en `ollama` dentro de `config/defaults.json` y se sobrescriben mediante la configuración validada.

| Grupo | Valores configurables |
|---|---|
| Proveedor | URL local, modelo, contexto, generación, tiempo de carga y de respuesta |
| Agentes | Máximo de pasos, subtareas, concurrencia y límites de contexto |
| Archivos | Raíces, exclusiones, tamaño máximo, codificación admitida y límites de búsqueda |
| Ejecución | Acciones de prueba/build registradas, programa/argumentos, directorio y timeout |
| MCP | Ejecutable, argumentos, entorno explícito, servidores habilitados y límites |
| GPU | Servidor compartido/exclusivo, espera de operaciones y consulta de memoria opcional |
| Historial | Directorio, límites de salida, retención y campos sensibles excluidos |

Propuesta inicial: una inferencia activa, hasta dos exploraciones nativas independientes y una aplicación de cambios a la vez. Contexto sugerido de 4096 tokens, configurable y validado contra capacidades del modelo; es un punto de prueba, no memoria ni velocidad garantizadas. Valores definitivos sujetos a mediciones y aprobación del plan.

## Ciclo modelo, herramienta y resultado

### Contrato E3/E4 implementado

`Files` aplica límites/exclusiones y operaciones Windows relativas a handles fijados: lectura UTF-8 con SHA-256, listado/búsqueda, creación sin reemplazo y edición de coincidencia única con backup y dos renames sin reemplazo. La instalación del ejecutor y áreas de control quedan protegidas. Se conservan owner/group/DACL; colisiones o verificación incompleta devuelven conflicto/partial con rutas observadas.

`Toolset` valida argumentos y permisos de rol, exige lectura previa para editar y ejecuta herramientas únicamente después del final válido del proveedor. El ejecutor mantiene hashes de archivos cambiados y congela las entradas leídas antes de una prueba. Las concesiones de comandos están fuera del proyecto y vinculadas a programa, argumentos, directorio y archivos observados; se revalidan al ejecutar y al cerrar tarea. El proceso se crea suspendido, se asigna a Job Object y se reanuda, con salida limitada y cancelación/timeout del árbol propio. No es un sandbox.

`Engine` serializa inferencias y herramientas y conserva conversaciones separadas para cinco roles. El coordinador planifica sin herramientas; explorador/reviewer solo lectura, implementador lectura/escritura/comandos concedidos y tester lectura/comandos concedidos. Los resúmenes son datos sin autoridad y se acotan. Para el reviewer, solo después de leer realmente todos los archivos cambiados y al menos un archivo se solicita `format` con esquema JSON `verdict` (APPROVE o CHANGES_REQUESTED) y `reason`, sin herramientas en ese paso. El cierre valida además las lecturas, el veredicto y la vigencia de la evidencia; un comando distinto no oculta fallos anteriores.

Los traspasos ordinarios entre roles usan hechos compactos de resultados reales del ejecutor, con registros JSON completos y recuento de omitidos dentro de `max_handoff_chars`; no reenvían indicaciones o hashes inventados en resúmenes del modelo. La razón de CHANGES_REQUESTED se transmite como solicitud de corrección del reviewer, sujeta a la misma política. Una respuesta sin actividad exigida que el ejecutor rechaza permanece como progreso visible y se retira del historial reenviado antes del feedback; los resultados reales de herramientas se mantienen.

El diario local JSONL sincroniza intención y resultado, con identificadores por sesión/rol/llamada, hashes y respaldos. Se reserva espacio antes de efectos y se detiene al agotar límites. Los resultados de herramientas permanecen visibles y en el diario aunque falle la inferencia posterior. Un estado parcial bloquea nuevos efectos y cierre de tarea durante la sesión; reiniciar exige inspección manual, sin reconciliación automática implementada. Los archivos de diario no son un historial de chat recuperable completo.

En tareas, un turno sin la actividad real exigida recibe feedback y continúa dentro de `agents.max_steps`. Implementador necesita creación/edición verificada o una inspección real con decisión JSON explícita de que no hacen falta cambios; esa decisión tampoco sustituye las pruebas. Ante prueba fallida o CHANGES_REQUESTED se vuelve a implementador/tester/reviewer con resultados observados. El mismo máximo limita las activaciones totales de roles, sin reiniciar el contador al reintentar. Los resultados de comandos se acumulan entre intentos; solo un reintento real del mismo id subsana su fallo anterior.

La UI añade `/permissions`, `/task` y `/chat`. La concesión requiere número y Ctrl+Enter sobre una propuesta vigente; Enter no concede. PgUp/PgDn desplazan y flechas fuera del menú recuperan peticiones. Las lecturas paralelas siguen pendientes. Ver [ejecución E3/E4](05-E3-E4.md) y [límites](03-PENDIENTES.md).

Cada agente mantiene instrucciones, mensajes y herramientas propias. El adaptador envía el historial completo necesario a `/api/chat`, reúne el flujo NDJSON y entrega el mensaje del asistente al orquestador. La ausencia de capacidades o un error se muestran como tales; no convertir bloques de código en ejecución implícita.

`ToolRequest`: identificadores de sesión/tarea/agente/llamada, herramienta, argumentos, versión de base cuando proceda y autorización efectiva. Los identificadores que falten en el proveedor se generan y conservan internamente.

`ToolResult`: llamada original, estado, rutas afectadas, datos observados, errores y evidencia. Estados: propuesta, autorizada, aplicada, verificada, fallida, cancelada, parcial o resultado desconocido. Un modelo no puede convertir fallo en éxito.

Los fragmentos incompletos del flujo nunca ejecutan herramientas. Una llamada completa necesita esquema, parámetros, ruta y permiso válidos. El resultado se devuelve al agente solicitante y la evidencia al coordinador. Límites agotados, errores dentro del flujo y desconexiones no finalizan como tarea completada.

## Archivos y concurrencia

- Leer devuelve contenido, límites/recortes y versión obtenida de los bytes; búsqueda y listados respetan exclusiones configuradas.
- Crear exige destino ausente y creación sin sobrescritura. Editar exige base conocida, parche exacto y comprobación de vigencia al aplicar; mover y borrar tienen también precondiciones y autorización.
- Coordinar todas las herramientas con efectos, incluidos comandos y MCP. El bloqueo del coordinador evita carreras entre sus propios agentes; no demuestra exclusión frente al usuario u otros procesos.
- En Windows se evaluarán handles con permisos de compartición restringidos y sustitución controlada. Si no se puede garantizar la publicación frente a una carrera externa, fallar o declarar conflicto; una comparación de hash seguida de escritura desprotegida no satisface el contrato.
- Temporales junto al destino, relectura posterior y preservación de Unicode, BOM y saltos de línea. No editar binarios como texto. Rechazar escrituras por enlaces/junctions no admitidos en v1.
- Una operación de varios archivos no es atómica por tratar cada archivo individualmente. Registrar exactamente los aplicados; recuperación que no pise ediciones posteriores. No prometer deshacer o recuperar un borrado sin implementación y prueba.

El diario local `.pillcli/sessions/` guarda intención, base, autorización y resultado, con límites y exclusión de secretos. Tras interrupción se reconcilia con disco: nunca reproducir automáticamente escrituras por volver a leer el historial. Primero se muestra el resultado parcial o desconocido.

## Comandos, permisos y MCP

Lectura y escritura se autorizan por la sesión dentro de raíces acordadas. Borrados, sustituciones destructivas y ampliaciones de alcance requieren confirmación visible del destino concreto. No pedir confirmación repetida para creación/edición ordinaria ya autorizada.

Pruebas y compilación se permiten automáticamente solo mediante acciones registradas por programa y argumentos. Descubrir un script en un repositorio no lo autoriza: el usuario debe registrar su definición antes de ejecutarlo automáticamente. Las acciones propuestas para aprobación de este proyecto se detallan en el plan. Un comando arbitrario requiere confirmación; no clasificar por prefijos de texto ni permitir encadenar comandos bajo el nombre de una prueba. Un script registrado puede tener efectos externos: no se promete sandbox. El cambio de su definición invalida la concesión; los efectos del código ejecutado por un build siguen sin estar aislados por esta política.

MCP: ejecutar únicamente servidores registrados explícitamente, sin `npx -y` ni descargas implícitas. Enumerar herramientas y validar esquemas con resolución de referencias externa deshabilitada. No fiarse de anotaciones de solo lectura como autorización. Las herramientas MCP reciben la misma política de efectos; los permisos del sistema operativo de un servidor externo no quedan limitados por el cliente. Sampling, ejecución de prompts como órdenes y transportes HTTP remotos no forman parte de esta entrega.

## `/liberate` e inicio

El mismo servicio atiende ambos disparadores, sin temporizador periódico. Bloquea nuevas admisiones, espera un punto seguro de las inferencias propias y comprueba propiedad/uso antes de solicitar descarga del modelo. Después confirma respuesta de Ollama y observa memoria disponible por separado.

Por defecto, servidor compartido: si no se puede demostrar exclusividad, conservar el recurso e indicar el motivo. `/api/ps` no identifica clientes activos y no prueba exclusividad. Un modo exclusivo exige configuración y confirmación explícitas del usuario, no se deduce porque la URL sea local. Al iniciar no se reclaman modelos de sesiones desconocidas ni se mata ComfyUI. Si NVIDIA no está disponible, informar de medición no disponible, no inventar VRAM.

## Fuentes técnicas

- [Ollama: herramientas](https://docs.ollama.com/capabilities/tool-calling), [errores](https://docs.ollama.com/api/errors).
- [RMCP oficial y cliente stdio](https://github.com/modelcontextprotocol/rust-sdk).
- [Ratatui](https://docs.rs/ratatui/0.30.2/ratatui/) y [Crossterm](https://docs.rs/crossterm/0.29.0/crossterm/).
- [Revisión previa de contratos](../REVISION-OLLAMA-ARCHIVOS.md).
