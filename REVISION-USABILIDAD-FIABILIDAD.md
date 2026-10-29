# Revisión de usabilidad, identidad y fiabilidad · 2026-10-27

Revisión solicitada por el usuario, sin modificar el producto. Se contrastaron
fuentes, capturas, configuración y evidencias del corte E5. Dos agentes revisaron
UI y fiabilidad por separado. Las prioridades siguientes son propuestas para
que el usuario decida alcance y diseño; no constituyen autorización para
implementarlas ni para instalar dependencias.

El binario revisado es `900f33be9fa88ba6445c2d66d7c117299f8cf11c37c6977b0cec188c968675f1`.
La evidencia existente acredita 119 pruebas locales, siete integraciones reales,
tester PASS y reviewer APPROVE en su alcance. No demuestra fiabilidad general
en cualquier proyecto. La edición ya exige lectura textual, base SHA-256,
publicación por handles, respaldo y relectura, y distingue resultados parciales.
Esas protecciones deben conservarse.

**Dos defectos de UI reproducidos en esta auditoría.** La sonda real
`evidencias/2026-10-27-auditoria-ui.json` comprobó que Esc deja abierto el selector
de modelos y que el pie de permisos sigue anunciando «enviar / Enter línea»
aunque Enter muestra la definición y Ctrl+Enter concede. No se generaron
respuestas, ejecutaron comandos ni concedieron permisos. El host propio terminó
con código 0 y restauró la consola. Este diagnóstico nuevo amplía la cobertura
anterior; no se presenta como un fallo ya corregido.

**Hallazgos confirmados por lectura y riesgos todavía no reproducidos.**
`Files::write_private` trunca `model.json` o `console.json` antes de reescribirlos;
los llamadores descartan errores de guardado. Un corte o disco lleno podría
perder conversación o dejar una versión antigua sin aviso. El recorte del
historial elimina mensajes individuales y la restauración adopta el sistema
guardado: puede romper una pareja assistant/tool o conservar instrucciones de
sistema antiguas después de cambiar configuración. Estos escenarios se deducen
del código; no se ejecutó una prueba nueva de corte de disco o energía.

| Prioridad propuesta | Mejora concreta | Beneficio y verificación esperada |
|---|---|---|
| Alta | Corregir Esc y mostrar atajos según el contexto: chat, modelos, permisos y confirmación | Cada tecla hace lo anunciado; Esc al cambiar modelo vuelve a la conversación previa. Probar teclado y pie juntos en consola real |
| Alta | Guardado resistente a interrupciones y errores visibles | Preparar/validar un temporal, publicar de forma controlada y conservar la última versión válida. Mostrar «conversación no guardada» y su causa. Probar fallo de escritura e interrupción entre pasos |
| Alta | Restauración y recorte por turnos completos | Conservar relaciones entre llamadas y resultados, versionar el formato y aplicar el sistema vigente. Probar historiales con herramientas y cambios de configuración |
| Alta | Recuperación guiada, sin desbloqueo ciego | Mostrar por operación la base, el estado actual, el resultado previsto y el respaldo; permitir resolver explícitamente lo revisado con SHA vigente y decisión durable. Lo no resuelto continúa bloqueado |
| Alta | Permisos escritos para personas | Mostrar «Mover origen → destino», «Eliminar archivo» o «Ejecutar prueba», con su estado. El ID de una operación lleva una huella larga que puede ocupar toda la fila; hashes y programa/argumentos quedan en un detalle legible |
| Alta | Cabecera compacta y detalles desplegables | La cabecera actual ocupa 10–11 filas, 12 con aviso de efectos. Mantener proyecto/modelo/modo/estado y trasladar digest, rutas largas y configuración a una vista de estado. Medir filas útiles en 73×21 |
| Media | Modos que expresen qué puede hacer el agente | El chat actual usa el implementador y puede editar. Proponer «Preguntar» de solo lectura, «Editar» y «Tarea con pruebas», con capacidades visibles y sin alterar permisos por un nombre de modo |
| Media | Diff de las ediciones y conflictos comprensibles | Mostrar líneas, antes/después, respaldo, coincidencias y BOM/CRLF. Permitir varios reemplazos exactos sobre una misma base y una publicación por archivo. No introducir coincidencias aproximadas silenciosas ni confirmaciones por cada edición ordinaria |
| Media | Deshacer una edición con precondición | Restaurar únicamente si el destino conserva el SHA posterior esperado; si cambió, mostrar conflicto y conservar ambos. No prometer deshacer universal, papelera ni atomicidad entre archivos |
| Media | Estados y errores orientados a la siguiente acción | Distinguir carga, razonamiento, herramienta, prueba, revisión y guardado. La línea de herramienta puede mostrar solo la ruta y omitir el motivo de error. Separar conexión fallida de contexto agotado o efecto desconocido; mostrar una acción pertinente para cada caso |
| Media | Redibujado incremental y caché de líneas | Hoy cada frame limpia y reconstruye la pantalla y vuelve a envolver todo el historial. Actualizar las filas cambiadas y limitar la frecuencia de render durante streaming; medir bytes escritos, CPU y latencia de teclado antes/después |
| Media | Perfiles de ejecución visibles y medidos | Mostrar contexto solicitado/observado, thinking y generación; permitir perfiles de chat y tarea elegidos por el usuario. Distinguir modelo seleccionado, cargado y perteneciente a la sesión |
| Media | Reducir trabajo repetido en lecturas y MCP | Reutilizar trabajadores en vez de construir hilo/runtime por lectura; evaluar catálogo/conexión MCP por huella, conservando revalidación y revocación. Añadir plazo de cierre y control del árbol propio. Medir primero; no cachear permisos como si fueran válidos para siempre |
| Media | Retención con aviso y archivo explícito | Mostrar espacio/cuota de sesiones y respaldos antes de agotarlos; permitir archivar los revisados con autorización y conservar los desconocidos. Añadir un límite de bytes al historial de pantalla |
| Media | Evaluación repetida de tareas de código | Corpus de varios archivos, CRLF/BOM, Unicode, contexto largo, pruebas fallidas y conflictos. Medir éxito completo, cambios indebidos y tiempo por perfil, con versiones y hashes; no elegir modelo por una sola muestra exitosa |

**Logo propuesto.** Mantendría la cápsula horizontal como identidad: mitad
rellena y mitad vacía, verde de marca y alternativa ASCII. El dibujo grande
puede aparecer al arrancar; durante el trabajo, una marca de una línea deja
más espacio para código. La actividad debe expresarse con texto y color,
con animación opcional solo durante trabajo y con frecuencia limitada.

Boceto de distribución, no captura de una funcionalidad implementada:

```text
(###|   ) pill   PILLCLITER | Tarea | modelo seleccionado
Implementador: editando src/main.rs | 12 s

  Respuesta, cambios y resultados de pruebas

> Describe la tarea...
Enter: nueva linea | Ctrl+Enter: enviar | / comandos
```

La vista de permisos tendría su propio pie: «Enter: detalle · Ctrl+Enter:
conceder/retirar · Esc: cerrar». La vista de modelos anunciaría elegir o
cancelar. El ancho de caracteres y grafemas necesita una política más precisa
que asignar dos columnas a todo carácter no ASCII; cualquier dependencia
adicional para ello requeriría autorización.
PgUp/PgDn debería usar el alto de historial que realmente muestra el frame,
en vez de restar una cantidad fija a las filas de la ventana. Como mejora
posterior, copiar un bloque de código o exportar una sesión facilitaría revisar
resultados sin depender de seleccionar texto dentro de la pantalla alternativa.

**Recursos: optimizar lo que cuesta.** La muestra anterior del CLI tiene
148 observaciones: máximo de working set de 9.379.840 bytes (8,95 MiB), privados
de 1.998.848 bytes y delta CPU de 0,53125 s. Son datos de esa prueba, no una
garantía ni un perfil nuevo. Conviene centrarse en contexto, generación,
representación del historial y arranque repetido de procesos. Aumentar el
contexto eleva la memoria necesaria, como indica la
[documentación oficial de Ollama](https://docs.ollama.com/context-length).
Propondría comparar perfiles sobre el equipo y su GPU compartida, manteniendo
la conservación de recursos ajenos y sin elegir automáticamente el modelo.

La actualización parcial de pantalla puede conservar el sistema actual de
salida VT, usando posicionamiento y borrado de líneas descritos por
[Microsoft](https://learn.microsoft.com/en-us/windows/console/console-virtual-terminal-sequences).
Para persistencia, Windows ofrece reemplazo con respaldo mediante
[ReplaceFileW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew);
habría que evaluar su compatibilidad con los handles fijados y los estados de
error antes de elegirlo. No se propone sustituir sin más la publicación segura
de los archivos del proyecto.

**Orden recomendado para aprobación.** Primero, corregir los dos defectos
visibles y endurecer guardado/restauración. Después, cabecera y logo compactos,
diff y recuperación guiada. Finalmente, optimización medida del render,
trabajadores, MCP y perfiles. Cada bloque necesita criterios de aceptación y
pruebas reales; las cifras de ahorro y fiabilidad se publicarán solo después
de medirlas.

Referencias locales principales: `src/ui/model.rs`, `src/ui/render.rs`,
`src/ui/native.rs`, `src/files/mod.rs`, `src/session/mod.rs`,
`src/tools/recovery.rs`, `src/tools/mcp.rs`, `src/tools/mod.rs`,
`evidencias/2026-10-27-tester.md` y `evidencias/2026-10-27-review.md`.
