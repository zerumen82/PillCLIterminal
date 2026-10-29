# Pendientes y riesgos

Seguimiento 2026-10-27: implementación aprobada de R09, lecturas nativas concurrentes y reconciliación de diarios, con 119 pruebas locales, siete integraciones opt-in explícitas, tester PASS y reviewer APPROVE del corte E5 y ampliaciones. Ver [contrato del corte](09-CIERRE-20261027.md). El estado del 20/22 de octubre que sigue es histórico y no describe la ausencia actual de esas funciones. E6 conserva las decisiones de modelo principal, PATH persistente y límites de pegado/glifos que requieren resolución explícita.

Estado: 2026-10-20. Base E1/E2, herramientas nativas E3/E4 y muestra de corrección con cinco roles verificadas, tester PASS y reviewer APPROVE. PATH de sesión verificado; inscripción persistente bloqueada por permisos del Registro. El resto de los requisitos de v1 se conserva a continuación.

Continuación E3/E4: archivos, ejecución de comandos y muestra de cinco roles con tester PASS/reviewer APPROVE; evidencia en `spec/05-E3-E4.md`. Las lecturas del producto todavía se serializan: R05 permanece parcial hasta verificar dos lecturas independientes concurrentes. Mover/eliminar (R09) y recuperación automática siguen pendientes. R04 conserva la interrupción de stream de herramientas en integración real pendiente; sus fragmentos inválidos están cubiertos por parser y el ciclo válido por inferencia real.

Continuación E5, 2026-10-22: el cliente MCP stdio y `/liberate` están implementados y cubiertos por pruebas locales. Contrato en `spec/06-E5.md`. El cierre independiente no se ha ejecutado, así que R12 y R13 permanecen parciales. No se conectó un servidor MCP elegido por el usuario ni se descargó un modelo de su GPU.

Continuación de consola, 2026-10-22: Ctrl+R abre el razonamiento plegado, `/journal` lee el diario existente, la barra muestra los segundos, el borrador borra un grafema y la conversación se guarda en `.pillcli`. Eso no reconcilia efectos parciales. E5 y v1 siguen sin cierre independiente.

Límites declarados E3/E4: la cota conservadora de bytes de esquemas/historial puede agotar el contexto 4096 tras pocas lecturas; `ollama.num_ctx` es configurable y el diagnóstico admite `PILL_LIVE_CONTEXT`. Un efecto parcial bloquea más efectos durante la sesión y exige inspección manual de diario/respaldos antes de reiniciar; todavía no hay reconciliación automática. Las pruebas revalidan archivos leídos/cambiados y definiciones observadas, sin prometer seguimiento de dependencias transitivas que no se hayan leído/declarado. La salida de consola por caracteres no garantiza todos los grafemas; propuestas de comandos numerosas/largas pueden requerir desplazamiento y conviene revisar su definición completa antes de conceder. El diario conserva hashes/resultados y no un historial recuperable completo.

## Antes de programar

| ID | Pendiente | Cierre verificable |
|---|---|---|
| P01 | Plan técnico y dependencias aprobados; precisión de nombre `pill` y consola registrada | Cerrado por respuesta del usuario; no cambia la necesidad de aprobar modificaciones posteriores |
| P02 | Licencias y compatibilidad de dependencias futuras | E3/E4: 93 paquetes externos en árbol Windows, licencias declaradas permisivas y build Rust 1.94 verificado offline. E5 añade `rmcp` 3.6.0 (Apache-2.0, features `client` y `transport-child-process`) y `jsonschema` 0.59.0 (MIT, sin resolución HTTP ni de archivos). El primer `cargo check --locked` necesitó red para bajar transitivas ausentes de la caché; después, formato, Clippy, pruebas y release usaron `--locked --offline`. El 2026-10-27 se inventarió el árbol Windows completo: 160 paquetes externos, todos con licencia declarada; evidencia en `../evidencias/2026-10-27-licencias.json`. No se elige licencia del proyecto. Otras plataformas no están acreditadas |
| P03 | Enlace MSVC | Cerrado para E1: compilación release y ejecución del binario real correctas, sin actualizar toolchain |
| P06 | Acceso a dependencias | Cerrado para interfaz: el usuario eligió Win32 nativo con caché disponible. Ratatui/Crossterm/Clap retirados. La falta de conexión sigue siendo un riesgo para dependencias futuras no presentes |
| P07 | Registro de pill en PATH de usuario | CERRADO 2026-10-28 con autorizacion expresa, instalador revisado, 49tests independientes y registro real PASS. Preserva valor/tipo/entradas previas y Machine PATH; manifiesto acredita entrada propia. Desinstalacion real fuera de esta autorizacion. Evidencia: `../evidencias/2026-10-28-instalacion-user-real-segundo.json` |
| P08 | Pegado multilínea y glifos | Ctrl+Enter envía peticiones y Enter añade línea: PASS real E2. Comandos del menú conservan Enter. Retroceso, Supr, izquierda y derecha mueven o borran un grafema local (ZWJ, marcas, selector, tono, tecla y bandera) sin biblioteca nueva; el salto por palabras sigue por espacios. El ancho no ASCII sigue siendo conservador. Una captura antigua de conhost mostró U+FFFD al leer un emoji; esta entrega no acredita todos los glifos. La política adicional de pegado del menú sigue pendiente |
| P04 | Decidir modelo principal mediante mediciones | Tres muestras reales Huihui completadas: ver comparación E2. Selección principal pendiente del usuario; los tiempos incluyen cargas/cachés distintas y no acreditan calidad general |
| P05 | Publicación de archivos frente a cambios externos | Verificada para archivo individual mediante pruebas Windows de carrera, bloqueos, preservación DACL/BOM/CRLF, colisión, rollback y resultado parcial; no hay transacción de varios archivos |

## Riesgos que condicionan la implementación

- GPU compartida: ComfyUI mantiene memoria. No detenerlo ni liberar recursos ajenos para hacer pasar una prueba. Un modelo puede utilizar CPU/RAM y cambiar latencia; mostrar lo observado.
- Las etiquetas de modelos pueden cambiar: registrar digest. Abliteration y anuncio de `tools` no garantizan calidad, argumentos correctos ni ausencia total de rechazos.
- Serializar agentes no resuelve por sí solo cambios concurrentes del usuario. No anunciar garantía de preservación si la publicación real no la cumple.
- El contexto máximo anunciado no es contexto viable en 8 GiB. Evitar cargar historiales de todos los agentes en cada petición.
- Un MCP `stdio` o comando hijo tiene acceso real del usuario al equipo. La política del CLI controla invocaciones, no constituye aislamiento del proceso.
- Configuración de repositorio no confiable: separar valores de propuestas de permisos y concesiones privadas. Probar que editar configuración no activa comandos, MCP, nuevas raíces ni exclusividad, y que cambiar una definición invalida la concesión asociada.
- Servidor Ollama compartido: `/api/ps` no permite identificar todos los usuarios. Conservar modelos si no se puede verificar propiedad; el modo exclusivo requiere concesión explícita.
- Un diario puede quedar incompleto tras un cierre abrupto. Recuperar como estado desconocido antes de reintentar; no prometer transacciones globales ni deshacer universal.
- No hay un servidor MCP de usuario elegido. La capacidad se verificará con un proceso local de pruebas; conectar o instalar servidores externos concretos sigue requiriendo autorización.
- El `link.exe` encontrado en PATH pertenece a Cygwin. Se localizaron Visual Studio Build Tools mediante `vswhere`; validar que Cargo utiliza herramientas MSVC correctas, no asumirlo por el nombre del ejecutable.
- El control de entrada verifica terminales y cantidad de procesos de consola, no identidad del padre ni antigüedad. Se han observado rechazo de consola propia y aceptación en PTY; no se promete una detección universal de cómo se lanzó cada proceso.

## Hallazgos de usabilidad y fiabilidad · 2026-10-27

Auditoría posterior al corte E5, solicitada para revisar y sugerir mejoras.
No se modificó el producto. Propuestas y prioridades pendientes de decisión
en [REVISION-USABILIDAD-FIABILIDAD.md](../REVISION-USABILIDAD-FIABILIDAD.md).

- Defectos reproducidos en consola real: Esc no cierra el selector de modelos;
  el pie de permisos anuncia enviar/nueva línea cuando las acciones reales
  son conceder/retirar y mostrar detalle. Evidencia nueva:
  `../evidencias/2026-10-27-auditoria-ui.json`.
- Riesgos confirmados por lectura, sin nueva prueba de corte/disco lleno:
  guardado de conversación por truncado y errores descartados; recorte por
  mensajes individuales y restauración de instrucciones de sistema antiguas.
- Mejoras propuestas: recuperación guiada, permisos y diff legibles, cabecera
  y cápsula compactas, modos con capacidades explícitas, cálculo de ancho,
  render incremental, cuotas visibles y evaluación de fiabilidad más amplia.
  El ahorro de recursos y el aumento de éxito deben medirse antes de afirmarlos.

## Fuera de la primera entrega aprobada

Búsqueda web y MCP remoto HTTP; instalación automática de dependencias/modelos/servidores; borrado recursivo; sandbox Docker/WSL; worktrees para editar en paralelo; comparación con otros motores antes de disponer de una línea base.

Estas exclusiones no eliminan la búsqueda web del objetivo futuro: el usuario aprobó hacerla después de validar el ciclo local.

## Estado documental existente

El 2026-10-22 el usuario pidió alinear la documentación con el repositorio. `AGENTS.md` describe la estructura real de `pill` 0.1.0, los comandos de comprobación y los límites de v1. La instrucción anterior de conservarlo como andamiaje vacío queda sustituida por esa petición. Estas especificaciones siguen sin inventar comandos que no existen.

`PLAN-ORIGINAL.md` sigue siendo referencia histórica: el prefijo externo de su H1 se conserva y la revisión inicial de Ollama es una instantánea anterior a los modelos Huihui. El plan técnico y las especificaciones aprobadas gobiernan la implementación. El seguimiento de etapas distingue E1–E4, cerradas en su alcance, de E5, implementada y con reviewer APPROVE del código 2026-10-27, con tester PASS independiente del mismo corte, y de E6, todavía pendiente por P04/P08; P07 se verifico el 2026-10-28.


## Cierre de la mejora integral 2026-10-28

Las propuestas de la auditoria anterior fueron aprobadas en U01-U15 e
implementadas con tester PASS y reviewer APPROVE sobre fuentes finales.
P07 queda cerrado; P04 y P08 requieren decision del usuario. La instalacion
real rechazo inicialmente el hardlink normal de Cargo sin alterar datos;
se corrigio solo la lectura del artefacto y se conservaron ambos FAIL y PASS.
Quedan como limites declarados la copia global al portapapeles no ejercitada,
la descarga GPU exclusiva no repetida y ausencia de una promesa general de
ahorro de RAM/CPU o de corte fisico de energia. El informe de recursos final
registra incrementos de memoria en la muestra; no los presenta como ahorro.


## Pulido posterior1y2 · verificado

Cerradas la distinción revisada/pendiente, contador de recuperación y omisión
del atajo de perfiles. Render selecciona referencias a caché y copia su ventana;
162tests,21checksUI y medición explícita PASS, con reviewer APPROVE. La CPU
observada bajó en la prueba de historial largo; RAM variable no justifica
promesa general. Portapapeles aislado(3) y especificación adicional de pegado(4/P08)
quedan fuera de esta entrega aprobada1y2. P04/P08 conservan su decisión pendiente.
