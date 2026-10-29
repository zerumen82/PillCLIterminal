# Requisitos y aceptación

Seguimiento vigente del 2026-10-27: los cuadros E2/E3/E4/E5 que siguen son
históricos. El usuario aprobó completar los pendientes según
[el contrato del corte](09-CIERRE-20261027.md). Release y distribuido actuales:
SHA-256 `900f33be9fa88ba6445c2d66d7c117299f8cf11c37c6977b0cec188c968675f1`.
Formato, Clippy, 119 pruebas locales, siete integraciones opt-in explícitas y build offline PASS. Reviewer independiente APPROVE en `../evidencias/2026-10-27-review.md`; tester independiente PASS en `../evidencias/2026-10-27-tester.md`, auditoría 57/57 comprobaciones.

R09 incorpora mover archivos sin sobrescribir y borrar archivos/carpetas vacías
con huella de raíz/ruta/destino/SHA/identidad y concesión explícita de un uso.
La inspección de metadatos no permite editar sin leer el texto. R05 añade lotes
de lecturas nativas en hilos acotados; dos lecturas reales de 8 MiB se solaparon
865.029.500 ns en hilos distintos, con evidencia en
`../evidencias/2026-10-27-lecturas-paralelas.json`. R08/R14 reconcilian diarios
al arrancar y distinguen aplicadas, pendientes y desconocidas, sin repetir ni
restaurar operaciones; los efectos desconocidos siguen bloqueados tras reiniciar.
Los lotes conservan resultados por archivo, sin atomicidad entre archivos.

La consola reducida a 73×21 y las 21 comprobaciones adicionales de permisos,
edición, scroll, persistencia y recuperación pasaron de forma independiente en
`../evidencias/2026-10-27-tester-consola-basica-r3.json` y
`../evidencias/2026-10-27-tester-funciones-consola-r3.json`.
R04/R12/R13 y E5 quedan verificados en el alcance del corte final. E6/v1 no se cierra: P04, P07 y la decisión adicional de P08 siguen pendientes del usuario; las limitaciones de glifos se conservan explícitas.

Estado: plan aprobado, 2026-10-20. Base E1/E2, herramientas nativas y muestra de corrección E3/E4 con tester PASS y reviewer APPROVE. R01/R06/R07/R11 verificados en Windows y R10 en la muestra/perfil especificados. R02/R03/R04/R05/R08/R14/R15 parciales; R09/R12/R13 pendientes. Registro persistente de PATH y elección de modelo principal pendientes. Los cuadros posteriores se conservan como históricos; la cabecera de 2026-10-27 recoge el corte vigente. No se declara v1 completa.

Evidencia E1: `../evidencias/2026-10-20-e1-consola.json`. Incluye eventos Win32 sobre el binario real invocado como `pill`, menú sin Enter, filtro, Esc, Tab, edición Unicode, redimensionado y restauración de modos/cursor/atributos tras `/exit` y Ctrl+C. No acredita inferencia. La inscripción persistente del comando en PATH fue bloqueada por permisos del Registro; funciona con PATH de sesión.

## R01 · Terminal y cápsula

Invocar `pill` desde PowerShell/Windows Terminal y mostrar cápsula propia, proyecto, modelo y estado. La interfaz requiere consola interactiva existente; arranque sin consola, con entrada/salida redirigida o mediante consola propia de doble clic se rechaza antes de abrirla. No se crea lanzador gráfico. Ayuda y versión pueden consultarse sin iniciar interfaz. Responder progresivamente sin bloquear entrada, manejar redimensionado y restaurar terminal al salir o cancelar. Texto de producto en español; nombres técnicos conservados.

Aceptación: ejecutar el binario real en terminal, introducir una petición, redimensionar y cancelar. Registrar evidencia visual y comportamiento de salida. Diseño propuesto: cápsula verde en caracteres, con alternativa ASCII si la terminal no representa el dibujo.

## R02 · Menú `/`

Escribir `/` como inicio de comando en la entrada abre inmediatamente el listado sin Enter; muestra comandos realmente disponibles, incluido `/liberate`, con descripción. Una barra dentro de una ruta o texto ordinario no dispara ejecución.

Propuesta de interacción para aprobar: filtrar mientras se escribe, flechas para seleccionar, Enter para ejecutar y Esc para cerrar conservando entrada. Registro inicial: `/help`, `/liberate`, `/exit`. Mostrar el menú no ejecuta nada. Comprobar con entrada real y evidencia visual; no solo una prueba del renderizador.

## R03 · Proveedor y modelos

Continuación E2 autorizada mediante «sigue» el 2026-10-20. Protección de pegado elegida explícitamente por el usuario: Ctrl+Enter envía peticiones; Enter añade línea. Los comandos del menú conservan Enter para ejecutar. La selección inicial es explícita mediante lista local o `--model`/configuración; nunca elegir automáticamente el primer modelo. Cambiar de modelo inicia una conversación nueva. Esc cancela una petición activa; Ctrl+C sale restaurando la consola. Fallos/cancelaciones no añaden respuestas parciales al historial enviado al modelo. E2 no ejecuta herramientas ni descarga/libera modelos.

Parámetros E2 en `ollama`: URL HTTP loopback, modelo opcional, contexto, límite de generación, temperatura, thinking, tiempos, tamaños y prompt de sistema. Rechazar redirecciones, proxies y endpoints remotos. Los valores incluidos no fijan un modelo principal. Validar contexto contra `/api/show` y digest contra `/api/tags`; conservar métricas reales sin presentar contexto solicitado como efectivo.

Descubrir modelos y capacidades locales, elegir uno explícitamente y mostrar su nombre/digest. No descargar ni sustituir automáticamente. Conexión fallida, modelo ausente o sin herramientas producen diagnóstico específico.

Aceptación: conectar a Ollama real con un modelo Huihui instalado; registrar configuración, digest, contexto efectivo si el runtime lo expone, tiempos, memoria y errores. Comparar los tres modelos antes de elegir el principal; no atribuirles resultados del modelo original ni del tamaño mayor.

## R04 · Ciclo de herramientas real

El modelo solicita lectura de una muestra local, el ejecutor la lee y devuelve sus datos; el modelo utiliza esos datos. Vincular agente/llamada/resultado. Rechazar herramientas desconocidas, argumentos incorrectos y llamadas parciales sin efectos.

Aceptación: inferencia real y archivos observados. Interrumpir streaming y comprobar que no se ejecutan argumentos incompletos. Un texto «he leído» no pasa la prueba. Pruebas deterministas de fallos del parser complementan, pero no sustituyen, esta integración.

## R05 · Coordinación multiagente

Coordinador asigna al explorador, implementador, tester y reviewer; muestra estados e integra resultados con evidencia. Compartir modelo no mezcla historiales. La falta de subtarea necesaria deja trabajo pendiente, no PASS.

Aceptación: una tarea de programación usa roles distintos con registro separado, respeta límites configurados y puede ejecutarse con una sola inferencia activa. Dos lecturas independientes avanzan sin bloquear entre sí; las modificaciones se coordinan.

## R06 · Lectura y búsqueda

Listar, buscar y leer archivos reales dentro de raíces acordadas, incluyendo nombres con espacios/Unicode. Informar archivos omitidos por límites o exclusiones; no declarar búsqueda exhaustiva si fue parcial. No requiere Git.

## R07 · Creación, escritura y edición

Crear una muestra, editar mediante cambio exacto y releer bytes. Intentar crear sobre destino existente devuelve conflicto y conserva su contenido. Preservar CRLF/BOM y texto no solicitado. Archivo binario, bloqueado, sin permisos o fallo de escritura devuelven error concreto sin éxito ficticio ni original truncado.

Aceptación: muestras temporales reales y casos de error reproducidos; comprobar que el cambio depende de bytes leídos. La autorización ordinaria de lectura/escritura no pregunta por cada archivo dentro del alcance.

## R08 · Conflictos y cambios parciales

Modificar una base entre lectura y aplicación, y dos agentes sobre el mismo archivo: detectar conflicto o integrar de forma explícita, sin perder cambios. Comprobar también la ventana de carrera durante publicación. Interrupción en un lote informa archivos aplicados y pendientes sin restaurar encima de trabajo nuevo del usuario.

## R09 · Mover y eliminar

Propuesta para v1: mover/renombrar sin sobrescribir destinos y eliminar un archivo o carpeta vacía tras confirmación concreta. Borrado recursivo queda pendiente de una ampliación de alcance. Esta propuesta no autoriza operaciones sobre archivos del usuario durante el desarrollo.

Aceptación: usar únicamente muestras descartables; rechazo sin autorización, destino cambiado, archivo bloqueado o enlace no admitido. Tras autorización se verifica estado del origen/destino o ausencia. No prometer papelera ni recuperación automática no probadas.

## R10 · Corregir un programa

Sobre un proyecto de prueba real: reproducir fallo, leer, proponer/aplicar corrección y ejecutar comprobaciones pertinentes. Registrar antes/después y versión exacta. Un cambio posterior invalida PASS/APPROVE afectados. Detener bucles que no progresan según límites configurados.

## R11 · Ejecución de comandos

Pruebas/build registrados se ejecutan con programa, argumentos y directorio visibles; capturar stdout/stderr acotados, código y timeout. Comando arbitrario, instalación o acción destructiva requiere confirmación. Un nombre de script no otorga permisos.

Aceptación: ejecutar un comando permitido y otro que falla, cancelar un árbol hijo propio y comprobar el resultado; un intento de encadenar una segunda orden no hereda aprobación. No cerrar procesos ajenos.

Prueba de confianza de configuración: abrir un repositorio con `.pillcli/config.json` que propone una raíz externa, un comando, un servidor MCP y modo exclusivo. Sin concesiones previas, debe mostrar las propuestas y no acceder/ejecutar/ampliar exclusividad. Aceptar solo una concesión no aprueba las otras; modificar la definición aprobada invalida su autorización. Los agentes no pueden editar el almacén de concesiones para eludir esta comprobación.

## R12 · Cliente MCP `stdio`

Conectar a servidor local registrado, negociar protocolo, listar e invocar herramienta real y mostrar errores de protocolo o herramienta. Validar entrada y aplicar política antes de invocar. Arrancar con MCP desactivado debe seguir permitiendo trabajo nativo.

Aceptación: servidor de prueba real como proceso hijo, sin descargas implícitas. Mostrar error ante esquema malformado o referencia externa; no resolverla por red o leyendo archivos arbitrarios. Cortar el servidor durante una llamada no produce éxito.

## R13 · Liberación de recursos

Al iniciar y mediante `/liberate`, intentar liberar recursos sin interrumpir trabajo ni clientes ajenos. En servidor compartido o de propiedad incierta, conservarlos e informar del motivo. Sin limpieza periódica ni promesa de VRAM cero.

Aceptación: verificar que ComfyUI/procesos ajenos permanecen. Probar por separado servidor compartido y un recurso exclusivo realmente autorizado; coordinar nueva inferencia concurrente y registrar solicitud, confirmación del servidor y VRAM observada. Si solo se ha probado el caso de conservación, la descarga efectiva sigue pendiente.

## R14 · Historial y recuperación

Registrar operaciones locales con base, autorización, resultado y evidencia; limitar retención y datos sensibles. Reiniciar tras una respuesta perdida distingue operación no realizada de aplicada o desconocida. Reabrir la sesión no repite efectos automáticamente.

## R15 · Calidad y recursos

Pasar `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked` y `cargo build --release --locked`, una vez exista proyecto y lockfile. Las pruebas de integración real que requieran Ollama se ejecutan expresamente y su omisión no cuenta como PASS.

Medir RAM/CPU del CLI por separado de RAM/VRAM e inferencia de Ollama. Registrar consumo en reposo y durante una tarea. No hay todavía umbral aprobado ni comparación que permita prometer ahorro numérico.

## Trazabilidad de aceptación

Cada R01–R15 necesita estado, versión/digest, comando o interacción, resultado, evidencia y veredictos. Las evidencias de instalación acreditan presencia de modelos, no criterios funcionales.

### Cierre de la etapa E2

Versión: `pill 0.1.0`; binario compilado/distribuido SHA-256 `63C9F211A44DC41FD9D55CD0892C37AC3ACACCC6E02A17E7936944C3494F4EE7`. Fuentes y comandos reproducibles en [review](../evidencias/2026-10-20-e2-review.md) y [tester](../evidencias/2026-10-20-e2-tester.md): APPROVE y PASS independientes. Formato, Clippy con warnings denegados, 14 pruebas locales y build release offline en verde. Las tres pruebas opt-in se ejecutaron expresamente; el chat se comprobó por separado para los tres modelos.

Evidencias: [comparación y digests](../evidencias/2026-10-20-e2-comparacion.json), [cancelación y reintento](../evidencias/2026-10-20-e2-cancel-retry.json), [errores reales](../evidencias/2026-10-20-e2-errors.json), [consola con Ollama](../evidencias/2026-10-20-e2-consola.json) y [regresión de consola](../evidencias/2026-10-20-e2-regresion-consola.json). Contexto efectivo observado: 4096 en los tres candidatos. Selección, pegado sin envío, Ctrl+Enter, nueva conversación, entrada/redimensionado durante generación, Esc y restauración comprobados mediante eventos Win32 y capturas reales.

| Requisito | Estado tras E2 | Evidencia o límite |
|---|---|---|
| R01 | VERIFICADO E1–E2 | Consola real y chat; PASS/APPROVE sobre la versión indicada |
| R02 | PARCIAL | Menú `/help`, `/model`, `/exit` real; `/liberate` pendiente |
| R03 | PARCIAL | Chat, selección, streaming, errores y comparación PASS; principal aún sin elegir |
| R04 | PENDIENTE | Ciclo de herramientas no implementado |
| R05 | PENDIENTE | Agentes del producto no implementados |
| R06 | PENDIENTE | Herramientas de lectura/búsqueda no implementadas |
| R07 | PENDIENTE | Herramientas de escritura/edición no implementadas |
| R08 | PENDIENTE | Publicación/conflictos de archivos sin verificar |
| R09 | PENDIENTE | Mover/eliminar no implementados |
| R10 | PENDIENTE | Corrección autónoma de programa no probada |
| R11 | PENDIENTE | Ejecutor de comandos del producto no implementado |
| R12 | PENDIENTE | MCP del producto no implementado |
| R13 | PENDIENTE | Liberación no implementada; E2 no solicita descargas |
| R14 | PENDIENTE | Conversación E2 en memoria; sin recuperación persistente |
| R15 | PARCIAL | Calidad E1–E2 y recursos medidos; faltan regresiones/integraciones de E3–E6 |

Las muestras generan texto de una función pero no lo ejecutan. Recursos CLI/Ollama/GPU se registran por separado en los diagnósticos del tester, sin umbral ni promesa de ahorro. El servidor sigue compartido; no se solicitan descargas ni se detienen procesos ajenos. P07/P08 y las demás limitaciones permanecen en pendientes.

### Continuación E3/E4 — 2026-10-20

El cuadro E2 anterior se conserva como histórico. Versión actual `pill 0.1.0`, SHA-256 release/distribuido `4FB5156349D322C64277C6BF299B6BF9009CE5090AB8DED0DC0A3DEC023204FA`. [Comprobaciones técnicas](../evidencias/2026-10-20-e34-checks.json): formato, Clippy, 48 pruebas locales y build release offline PASS; 93 paquetes externos con licencia declarada revisada. [Reviewer independiente](../evidencias/2026-10-20-e34-review.md): APPROVE de código sobre hashes concretos. [Tester independiente](../evidencias/2026-10-20-e34-tester.md): PASS de consola completa, 26/26 checks, y regresión E1 sobre ese mismo binario.

La [consola final](../evidencias/2026-10-20-e34-consola-completa.json) usa `huihui_ai/qwen3-abliterated:4b-v2`, contexto 16384, thinking=true y generación máxima 2048. La prueba falla con código 1 antes, el implementador lee y edita `calc.py`, el tester ejecuta `test_suma` con código 0 y el reviewer lee el SHA final y devuelve APPROVE. Las pruebas no se modifican. Son roles/conversaciones separados, con inferencias seriales. La tarea observada duró 175,69 s y el chat posterior con lectura real 28,61 s; sin clasificación general de rendimiento ni modelo principal elegido. Permisos, Enter sin concesión, Ctrl+Enter concede, diarios correlacionados, salida y restauración: PASS. Recursos por proceso separados: CLI máximo observado 8.409.088 bytes de working set (8,02 MiB), 1.810.432 bytes privados y delta de CPU 0,5 s en 658 muestras; Ollama se registra aparte, sin umbral ni promesa de consumo constante.

Las muestras anteriores e intentos fallidos se preservan como históricos. El tester observó fallos con CRLF y LF, actividad inventada y finales vacíos; el ejecutor rechazó los falsos éxitos. Los parches siguen exigiendo coincidencia exacta; el feedback indica usar una subcadena literal mínima y conservar CRLF/BOM. La muestra autónoma final usa LF; conservación CRLF/BOM/DACL se verifica por separado en las pruebas Windows reales. Una muestra PASS no garantiza calidad general ni cierra requisitos pendientes de v1.

La consola final no produjo EmptyResponse ni la etapa `agent_completed_from_tool_evidence`. Esa rama tiene revisión y regresión unitaria específicas; no se afirma cobertura de integración real de su cierre con evidencia previa en esta consola.

| Requisito | Estado de este corte | Evidencia o límite |
|---|---|---|
| R01 | VERIFICADO E1–E4 nativo | Consola completa y regresión E1 PASS sobre SHA actual |
| R02 | PARCIAL | Menú añade `/permissions`, `/task`, `/chat`; `/liberate` pendiente |
| R03 | PARCIAL | Se conserva selección explícita/model=null; principal pendiente |
| R04 | PARCIAL | Ciclo nativo real PASS; fragmentos/duplicados inválidos rechazados en pruebas del parser; interrupción de stream con herramientas incompletas no verificada en integración real |
| R05 | PARCIAL | Cinco roles reales con permisos separados y límites PASS; lecturas independientes aún en serie |
| R06 | VERIFICADO herramientas nativas Windows | Listado/búsqueda/lectura reales, Unicode y límites/exclusiones probados |
| R07 | VERIFICADO herramientas nativas Windows | Crear/editar/releer, bases SHA, BOM/CRLF/DACL y destinos existentes probados |
| R08 | PARCIAL | Conflictos externos, carreras y reserva/rollback probados; no recuperación/lotes de varios archivos |
| R09 | PENDIENTE | Mover/eliminar no implementados |
| R10 | VERIFICADO en la muestra LF/perfil indicado | Corrección autónoma, prueba FAIL→PASS, revalidación y reviewer; tester independiente PASS |
| R11 | VERIFICADO ejecutor Windows | 12 integraciones reales: concesión, programa/argumentos, límites, cambios, timeout/cancelación del árbol; consola permisos PASS |
| R12 | PENDIENTE | MCP no implementado |
| R13 | PENDIENTE | `/liberate` y liberación al inicio no implementados |
| R14 | PARCIAL | Diario acotado de intenciones/resultados/hashes; recuperación automática e historial completo pendientes |
| R15 | PARCIAL | Calidad/consola E3/E4 y recursos observados PASS; criterios restantes de v1 pendientes |

### Continuación E5 — 2026-10-22

El cuadro E3/E4 anterior se conserva como histórico de aquel corte. Su tester PASS y su reviewer APPROVE no cubren este código. Release y `bin/pill.exe` son idénticos: SHA-256 `8FEA4F07FDC6F7532EB287EADDAD940A0AA6C6C25EF68E6F8E1E922B5490E9F5`, 12214784 bytes, 2026-10-22 14:17:21. Formato, Clippy con warnings denegados, 60 pruebas locales y build release offline PASS. Cinco pruebas opt-in de Ollama y herramientas con modelo siguen ignoradas. `python diagnostics/test_console.py` PASS sobre ese binario. No hay tester independiente ni reviewer de E5. No se eligió modelo principal, no se escribió el PATH de usuario y no se descargó ningún modelo de la GPU real: la liberación se comprobó contra un servidor HTTP en `127.0.0.1`.

| Requisito | Estado tras la comprobación local | Evidencia o límite |
|---|---|---|
| R01 | VERIFICADO E1–E4; menú E1 repetido en el binario E5 | La consola completa de cinco roles no se repitió sobre este SHA |
| R02 | PARCIAL | `/liberate` está en el menú y el filtro `e` sigue eligiendo `/exit`. La sonda no concede MCP ni ejecuta liberación |
| R03 | PARCIAL | `model` sigue `null` y los valores incluidos no cambian; principal pendiente |
| R04 | PARCIAL | Sin cambio respecto a E3/E4; la interrupción de stream con herramientas sigue sin integración real |
| R05 | PARCIAL | Sin cambio; las lecturas siguen en serie |
| R06 | VERIFICADO en el corte E3/E4 | Las pruebas de archivos siguen en verde; no se repitió la consola de cinco roles |
| R07 | VERIFICADO en el corte E3/E4 | Crear y editar siguen cubiertos por las pruebas Windows |
| R08 | PARCIAL | Sin cambio; no hay recuperación ni lotes de varios archivos |
| R09 | PENDIENTE | Mover y eliminar no implementados |
| R10 | VERIFICADO en la muestra E3/E4 | Esa muestra no cubre este binario |
| R11 | VERIFICADO en el ejecutor E3/E4 | Las 12 pruebas de comandos siguen en verde |
| R12 | PARCIAL | Proceso MCP real: una concesión no aprueba las demás, el eco concedido devuelve el texto, un `$ref` externo no publica herramienta ni lee `win.ini`, un cuelgue o una muerte no son éxito y un archivo fuera de la raíz no arranca. Falta el cierre independiente |
| R13 | PARCIAL | Stub local: el modo compartido no llama a `/api/generate`; con exclusividad se descarga solo el modelo seleccionado en la sesión y no se anuncia éxito si sigue en `/api/ps`; durante la liberación se rechaza otra petición. No se midió ni se vació la GPU real |
| R14 | PARCIAL | Diario sin cambio; recuperación automática e historial completo pendientes |
| R15 | PARCIAL | Comprobación local de E5 sin tester, sin reviewer y sin cierre de v1 |

### Continuación de consola — 2026-10-22

El cuadro E5 anterior conserva el SHA de aquel corte. El binario posterior, release y `bin/pill.exe`, es SHA-256 `FDD485C66C146517B3780FB65B065E646F38F345FE58A43259F7A3BEE3583127`, 12284416 bytes, 2026-10-22 22:48:41. Formato, Clippy con warnings denegados, 83 pruebas locales y build release offline PASS. Cinco pruebas opt-in siguen ignoradas. `python diagnostics/test_console.py --evidence evidencias/2026-10-22-ui-diario.json` PASS. Ctrl+R abre el razonamiento, `/journal` lee el diario real, la barra cuenta segundos, el borrador borra un grafema y `.pillcli/model.json` más `.pillcli/console.json` reabren la conversación solo con el mismo modelo. No se marcaron requisitos como verificados: no hay tester ni reviewer de este corte, `/model` no levanta efectos parciales y la recuperación no reconcilia escrituras. R14 sigue parcial. No es el cierre de v1.


### Estado vigente · 2026-10-27

Este cuadro sustituye los estados históricos anteriores, para el binario
`900f33be9fa88ba6445c2d66d7c117299f8cf11c37c6977b0cec188c968675f1`.
Tester PASS y reviewer APPROVE del corte E5 y ampliaciones; no cierra E6/v1.

| Requisito | Estado del corte | Evidencia y límites |
|---|---|---|
| R01 | VERIFICADO | Consolas básica, funciones y Ollama; entrada/resize a 73×21 y restauración real |
| R02 | VERIFICADO | Menú inmediato y permisos con Ctrl+Enter; liberación con backend real. P08 adicional pendiente |
| R03 | PARCIAL por decisión del usuario | Chat, streaming, digest y errores reales PASS; `model` continúa `null`, sin elegir principal |
| R04 | VERIFICADO | Herramientas reales y corte de bytes originales de Ollama antes del final: cero herramientas/intenciones/efectos |
| R05 | VERIFICADO | Cinco conversaciones/roles y dos lecturas reales solapadas en hilos distintos con límite configurable |
| R06 | VERIFICADO | Lectura/listado/búsqueda, límites y nombres Unicode; pruebas sobre disco |
| R07 | VERIFICADO | Creación/edición SHA, conflicto, bloqueo, preservación y regresión del hardlink externo |
| R08 | VERIFICADO en resultados por archivo | Conflictos y reconciliación de estados aplicados/pendientes/desconocidos sin pisar cambios; sin atomicidad global |
| R09 | VERIFICADO | Mover archivos sin reemplazar; borrar archivo/carpeta vacía, base/identidad y concesión concreta de un uso |
| R10 | VERIFICADO en la muestra LF/perfil | Dos opt-in y consola: defecto real, edición, prueba FAIL→PASS y reviewer con lectura/veredicto |
| R11 | VERIFICADO | Comandos registrados/concedidos, salidas/códigos reales y procesos propios; timeout/cancelación conserva incertidumbre |
| R12 | VERIFICADO | MCP stdio con hijo Python real, permisos separados, referencias cerradas, muerte/timeout/error con efectos desconocidos |
| R13 | VERIFICADO | Servidor propio exclusivo con GPU real y modo compartido conservador; modelos y procesos ajenos intactos |
| R14 | VERIFICADO en el contrato del corte | Recuperación acotada del diario al arrancar, sin replay ni rollback automático; persistencia de conversación separada |
| R15 | VERIFICADO técnicamente; E6 pendiente | 119 pruebas locales, siete opt-in, build/lint y controles independientes; recursos medidos sin umbral aprobado |

Evidencias adicionales: `../evidencias/2026-10-27-live-summary.json`,
`../evidencias/2026-10-27-consola-cinco-roles.json` (29 comprobaciones),
`../evidencias/2026-10-27-gpu-real.json` y su informe Rust. En GPU se usó el 2B
ya instalado, contexto 8192, generación 512 y thinking desactivado; acreditó
1.712.964.238 bytes del modelo en GPU antes de descargarlo. El servidor
compartido y cinco procesos Python ajenos conservaron sus identidades. La
VRAM global se mide por separado y su delta no se atribuye en exclusiva al modelo.
Las herramientas y los cinco roles usaron 4b-v2, contexto 16384, generación 2048
con thinking; es un perfil de prueba, no una elección de modelo principal.

El CLI tuvo 148 muestras: working set máximo observado 9.379.840 bytes,
privados 1.998.848 bytes y delta CPU de 0,53125 s. Ollama y los hosts se
registran por separado; no son un umbral ni promesa de consumo constante.


## Mejora integral 2026-10-28

Los criteriosU01-U15 aprobados se mantienen en
[10-USABILIDAD-FIABILIDAD-INSTALACION.md](10-USABILIDAD-FIABILIDAD-INSTALACION.md).
Controles independientes finales: formato/Clippy/releaseoffline y158pruebas
localesPASS,49comprobaciones de instalacion en muestrasPASS. Siete integraciones
realesPASS, incluidas dos tareas con cuatro archivos y respaldoSHA/BOM/CRLF.
SoloUI cambio despues; los modulos del ejecutor conservan los hashes probados.
La consola de mejoras tiene16comprobaciones y la legacy restaura estadosWin32.
La prueba de revision humana reconoce por separado los dos diagnosticos del
diario truncado: revisar uno conserva bloqueo del otro, sin repetir efectos.

Fuentes/evidencia: `../evidencias/2026-10-28-tester-independent-cuarto.json`,
`../evidencias/2026-10-28-live-summary.json` y
`../evidencias/2026-10-28-review-current-ui-final.json`.
El portapapeles global se preservo durante las pruebas; /copy solo se comprobo
sin respuesta y mediante revisionAPI. No se afirma prueba de corte fisico de
energia, ahorro universal deRAM/CPU ni representacion exacta de todosglifos.
El registroUser real PASS esta en `../evidencias/2026-10-28-instalacion-user-real-segundo.json`: valor/tipo previos conservados, Machine PATH intacto, proceso nuevo resuelve pill0.1.0 y simulacion de desregistro sin cambios.


## Pulido posterior aprobado1y2

[Contrato14](14-PULIDO-RENDER.md) implementado: contador y filas de revisión
conservan evidencia/estado, detalle de perfil anuncia la acción existente y
ventana de historial sin clonar toda caché. Formato/Clippy,162pruebas locales,
release offline, medición explícita y21checksWin32 a73x21 PASS independientes.
Reviewer APPROVE en `../evidencias/2026-10-28-review-polish.json`.
Binario62a1857654b2a15fc793029f2f54bc26e36e5ec815eb4e834d366180ae47f90f
instalado por SessionOnly conservando User/MachinePATH y propiedadUser.
No se modificó generación, permisos, formato de historial ni reconciliación.
