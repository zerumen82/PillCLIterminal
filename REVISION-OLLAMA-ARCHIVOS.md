# Revisión de Ollama, agentes y operaciones de archivos

Fecha: 2026-10-20. Estado: auditoría y propuesta técnica; no es una implementación ni la aprobación de un stack.

## Conclusión y alcance real

Ollama responde y permite consultar los modelos instalados. El repositorio todavía no contiene CLI, agentes, ejecutor de herramientas ni pruebas funcionales. Por tanto, no se ha demostrado que este producto cree, edite, corrija o elimine archivos. Las modificaciones documentales de esta sesión las realizan las herramientas de Codex; no son evidencia de capacidades de PILLCLI.

El modelo propone una operación; el programa valida sus argumentos, comprueba la autorización, la ejecuta y verifica el resultado. Generar código en una respuesta no modifica el disco. La API de creación/eliminación de modelos de Ollama tampoco crea/elimina archivos de un proyecto de programación.

## Evidencia local comprobada

Se consultaron `/api/version`, `/api/tags`, `/api/ps` y, para cada modelo, `POST /api/show`. Esta última consulta obtiene metadatos, no genera respuestas. Instantánea con fecha, digest y resultados: [evidencias/2026-10-20-ollama-metadatos.json](evidencias/2026-10-20-ollama-metadatos.json).

| Modelo exacto | Parámetros / cuantización | Contexto declarado en metadatos | Capacidades anunciadas |
|---|---|---:|---|
| `wizard-vicuna-uncensored:latest` | 7B / Q4_0 | 2048 | `completion` |
| `llama2-uncensored:latest` | 7B / Q4_0 | 2048 | `completion` |
| `llama3:latest` | 8.0B / Q4_0 | 8192 | `completion` |

El servidor declara versión `0.40.6`. No había modelos cargados antes ni después de la captura. No se ejecutó inferencia, no se descargó software ni se detuvieron procesos.

Ninguno anuncia `tools`; no se ha validado su uso con herramientas nativas. Esto no demuestra incapacidad para generar código. Tampoco acredita calidad de programación ni fiabilidad para controlar herramientas. El contexto declarado no es una medición del contexto realmente asignado en una inferencia. Los nombres y etiquetas no bastan para reproducir una prueba: registrar también digest y configuración.

## Interacción con Ollama: contrato propuesto

La documentación oficial describe enviar historial y herramientas a `/api/chat`, recibir `tool_calls`, ejecutar las funciones en el cliente y devolver sus resultados como mensajes de herramienta. En streaming deben reunirse las partes antes de continuar el ciclo. El servidor puede enviar un error dentro del flujo aunque la respuesta HTTP haya empezado correctamente. Fuentes: [herramientas](https://docs.ollama.com/capabilities/tool-calling), [chat](https://docs.ollama.com/api/chat), [errores](https://docs.ollama.com/api/errors).

Propuesta a trasladar a especificaciones tras aprobación:

1. Descubrir modelo y capacidades antes de asignarlo a un agente. Si falta compatibilidad, informar la limitación concreta; no sustituirla silenciosamente por comandos extraídos de texto libre.
2. Mantener historial separado por agente. Compartir servidor o modelo no equivale a compartir conversación: cada petición lleva su contexto y herramientas autorizadas.
3. Vincular sesión, tarea, agente y llamada mediante identificadores internos; no asumir que el proveedor aporta todos esos identificadores.
4. Validar nombre de herramienta, esquema, tipos, rutas, base del cambio y permiso antes de actuar. Una llamada inválida devuelve error sin efectos.
5. Acumular streaming, detectar finalización y errores, y rechazar llamadas incompletas. No ejecutar fragmentos de argumentos ni bloques de texto como si fueran acciones.
6. Devolver el resultado observado al mismo agente solicitante. El coordinador recibe rutas y evidencia verificable, además del resumen.
7. Distinguir operación propuesta, aplicada, verificada, fallida, cancelada y de resultado desconocido. Una afirmación posterior del modelo no cambia el estado registrado por el ejecutor.
8. Limitar pasos, tiempo, contexto y concurrencia mediante configuración. Conservar instrucciones y resultados necesarios al reducir contexto; declarar límites en vez de truncar datos relevantes silenciosamente.
9. No reintentar a ciegas efectos secundarios tras una desconexión. Consultar el registro y el estado real antes de decidir si la operación ocurrió.

Un JSON válido no demuestra una edición correcta. Una alternativa a herramientas nativas basada en salida estructurada requeriría contrato, validación y pruebas propios; no está aprobada ni implementada.

## Operaciones reales de programación: contrato propuesto

| Operación | Condición previa | Evidencia posterior necesaria |
|---|---|---|
| Leer | Archivo y ruta accesibles, límites explícitos | Contenido observado, codificación y versión de base; declarar recortes |
| Crear | El destino no existe y el alcance está autorizado | Creación sin sobrescritura accidental, relectura y contenido esperado |
| Editar o escribir | La versión actual coincide con la leída | Aplicación exacta, diff y relectura; conservar cambios ajenos y formato |
| Corregir un fallo | Problema reproducido o limitación documentada | Cambio real y comprobación pertinente sobre la versión modificada |
| Renombrar o mover | Origen conocido y destino sin colisión | Verificar destino y estado del origen; declarar operaciones parciales |
| Eliminar | Ruta concreta y autorización de borrado | Resultado del sistema y ausencia del destino; nunca inferir éxito del relato |

Los permisos de lectura/escritura concedidos evitan confirmaciones repetidas dentro del alcance acordado. Queda por concretar el alcance de rutas y la política de borrado. Las instrucciones globales exigen autorización antes de acciones destructivas; esta revisión no autoriza eliminar archivos del usuario. La recuperación de borrados necesita una decisión y pruebas específicas antes de prometer reversibilidad.

Requisitos propuestos para el ejecutor:

- Comprobar que la base sigue vigente al aplicar, no solo al preparar el cambio. Dos agentes pueden escribir en serie y aun así sobrescribir cambios si usan bases antiguas.
- Coordinar escrituras, borrados y comandos que modifiquen archivos. El control debe cubrir también cambios realizados por el usuario y por herramientas MCP.
- Definir cómo evitar la carrera entre comprobación y modificación; una comparación seguida de una escritura desprotegida no garantiza exclusión frente a procesos externos. Si la plataforma no permite garantizarlo, declarar la limitación.
- Escribir sin truncar el original ante fallos y definir publicación, durabilidad y recuperación. Atomicidad de un archivo no implica transacción de varios: informar exactamente qué quedó aplicado.
- Resolver rutas Windows, mayúsculas, enlaces, junctions, rutas de red y destinos fuera de alcance. No permitir que un enlace convierta una operación local en una modificación de otro árbol.
- Preservar Unicode, saltos de línea, BOM y contenido no solicitado. Tratar binarios de forma explícita; no pasarlos por edición de texto silenciosamente.
- Registrar errores reales: permisos del sistema, archivos bloqueados, espacio insuficiente, conflicto o parche sin coincidencia. No aplicar parches por aproximación sin una propuesta revisable.
- No devolver éxito por un resultado deseado. Releer contenido o comprobar el estado correspondiente tras ejecutar. Un diff acredita un cambio; las pruebas pertinentes acreditan comportamiento.

## Agentes, revisión y corrección

Explorador, implementador, tester y reviewer pueden compartir un modelo; sus herramientas, historiales y responsabilidades permanecen separados. El modelo local no aplica cambios por sí solo: los ejecuta el programa dentro de los permisos aprobados.

Las propuestas llevan la versión de base. Tester y reviewer deben vincular PASS/APPROVE al conjunto exacto de archivos comprobado; una modificación posterior invalida las comprobaciones afectadas. Un resumen de otro agente no sustituye salidas de comandos ni observación de archivos.

El ciclo de corrección propuesto es: observar problema, leer base actual, preparar cambio, aplicar, releer y verificar. Debe detenerse ante falta de progreso, conflicto, límite o error; no anunciar arreglos solo porque generó una explicación convincente. Cancelar o recuperar una sesión no puede reproducir escrituras o borrados antiguos.

## Pruebas reales pendientes

Estas son pruebas de aceptación propuestas, no resultados ejecutados:

1. Modelo exacto: seleccionar herramienta de lectura, recibir contenido real y usarlo en la respuesta; comprobar argumentos, errores y versión/configuración.
2. Crear un archivo en un proyecto de prueba, releerlo y comprobar bytes; repetir con destino existente y verificar que se conserva.
3. Editar un archivo con espacios/Unicode y formato CRLF/BOM; comprobar cambio solicitado y preservación del resto.
4. Cambiar la base después de la lectura y antes de aplicar: detectar conflicto sin pisar la nueva versión, incluso sin Git.
5. Dos agentes sobre el mismo archivo: integración explícita o conflicto; ningún cambio desaparece silenciosamente.
6. Provocar error de permisos, archivo bloqueado, escritura fallida y parche no aplicable; registrar fallo sin éxito inventado ni original truncado.
7. Tras aprobar borrados, eliminar únicamente un archivo de prueba; verificar ausencia, error por bloqueo y comportamiento ante enlaces. Nunca probar sobre archivos reales ajenos.
8. Cortar la comunicación después de aplicar: recuperar el estado sin repetir la operación. Interrumpir streaming antes de completar argumentos: ningún efecto.
9. Introducir un fallo reproducible en una muestra, corregirlo y ejecutar sus comprobaciones reales; vincular salidas y aprobación a los archivos finales.
10. Inicio y `/liberate` con inferencias en curso o clientes externos: coordinar nuevas solicitudes y conservar recursos compartidos o de propiedad incierta; distinguir confirmación del servidor de variación global de VRAM.

## Decisiones y riesgos pendientes

- Aprobar lenguaje, dependencias y modelo de pruebas antes de implementar o instalar. No hay evidencia para declarar un servidor más rápido en este equipo.
- Elegir alcance de rutas y política de eliminación; diferenciar archivo, carpeta vacía y borrado recursivo.
- Acordar persistencia del registro de operaciones, recuperación y garantías posibles frente a procesos externos.
- Redactar `spec/00-PRINCIPIOS.md` a `spec/04-EJERCITO.md` antes de implementar, con estos contratos y criterios aprobados.
- La propuesta original conserva un prefijo externo antes de su H1, observado cambiando durante la sesión; no se ha sobrescrito esa edición concurrente.

Resultado de la inspección local: consultas de metadatos correctas. Resultado funcional del producto: no evaluable, porque aún no existe implementación. Lint, compilación, regresiones y ciclo de herramientas de los modelos: no ejecutados.
