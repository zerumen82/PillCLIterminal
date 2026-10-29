# Mejora integral aprobada · 2026-10-27

El usuario autorizó «arréglalo todo» sobre
`../REVISION-USABILIDAD-FIABILIDAD.md` y pidió asegurar instalación y retirada
del PATH. Aprobó expresamente registrar `D:\PROJECTS\PILLCLITER\bin` mediante
el instalador revisado después de pasar las pruebas. Esto sustituye P07 solo
para ese registro; no autoriza borrar entradas ajenas ni desinstalar realmente.
No se incorporan dependencias, skills ni modelos nuevos; `model` sigue `null`.

Se aplican `fix` y `plan`. El diseño revisado previamente y su orden propuesto
quedan aprobados por la petición. Las decisiones técnicas rutinarias mantienen
los contratos de confianza, SHA, handles, respaldos y resultados desconocidos.

| ID | Cambio y rutas | Aceptación |
|---|---|---|
| U01 | Escape y pies contextuales en `src/ui/` | Esc cierra una selección sin cambiar modelo; cada pie anuncia la acción real. Ctrl+Enter confirma la descarga, Enter no concede ni descarga |
| U02 | Cabecera/cápsula compactas, estado y detalle en `src/ui/`, `assets/` | Conversación dispone de más filas; cursor, selección, avisos y footer visibles a 73×21. Identidad de cápsula y fallback ASCII |
| U03 | Permisos legibles, diff y detalle de resultados | Acción/origen/destino/estado visibles; huellas y argumentos en detalle. Los errores muestran causa, las ediciones verificadas muestran cambios observados |
| U04 | Modos explícitos Preguntar/Editar/Tarea | Preguntar carece de herramientas con efectos; cambiar modo no concede permisos. Se conserva `/chat` como compatibilidad de edición con herramientas |
| U05 | Guardado de archivos privados resistente a cortes en `src/files/` | Temporal validado y sincronizado, publicación controlada y última copia válida recuperable; hardlinks/reparse y destinos concurrentes se rechazan antes de alterar bytes externos |
| U06 | Historial versionado por turnos en `src/session/` | No quedan tool sin llamada ni turnos cortados; sistema vigente reemplaza al antiguo al restaurar. Errores de lectura/guardado visibles, sin éxito ficticio |
| U07 | Recuperación guiada y revisión durable | Mostrar operación, estado, versiones y respaldo. Reconocimiento humano ligado a evidencia vigente; no repetir efectos ni convertir desconocido en éxito. Lo no revisado conserva bloqueo |
| U08 | Varias ediciones exactas de un archivo y restauración condicionada | Base única, rechazo integral ante parche ambiguo y una publicación por archivo. Restaurar exige SHA posterior vigente y conserva evidencia; sin papelera ni transacción global |
| U09 | Errores por fase y consultas reintentables | Conexión, carga/primer dato, inactividad y límite total distinguibles y acotados por configuración. Solo consultas idempotentes se reintentan automáticamente; nunca efectos desconocidos |
| U10 | Render incremental, ancho y paginación reales | Filas sin cambios no se reescriben; historial envuelto reutilizable; PgUp usa el alto visible. Medir antes/después y preservar respuesta del teclado |
| U11 | Lecturas con trabajadores reutilizables y MCP acotado | Concurrencia configurada, sin runtime por lectura. Revalidar huellas/permisos aun al reutilizar catálogo; cierre de árbol propio y plazo de apagado. Registrar ahorro solo si se mide |
| U12 | Estado/perfiles y retención visibles | Separar seleccionado/cargado/propiedad y contexto solicitado/observado. Límites de historial por bytes; advertir cuotas y archivar solo evidencia revisada, con confirmación concreta |
| U13 | Instalación segura en `scripts/` | Detectar conflictos; preparar binario antes de publicar; validar hash/versión; preservar PATH y tipo/expansiones existentes; no tocar Machine PATH. Registro idempotente y manifiesto de propiedad |
| U14 | Desregistro seguro y simulación en `scripts/` | `-WhatIf` no cambia nada; retirar únicamente entrada normalizada que el instalador añadió y reconoce. Entradas preexistentes/ajenas se conservan. Borrar binario es opción separada, concreta y condicionada a identidad/hash; nunca borrado recursivo |
| U15 | Evaluación y cierre | Corpus de varias tareas/archivos/CRLF/BOM, fallos reales y recursos por proceso; formato, Clippy, pruebas, release offline, consolas reales, tester PASS y reviewer APPROVE sobre versión final |

Configuración nueva se declara y valida en el sistema del proyecto, conservando
compatibilidad mediante la mezcla con los valores incluidos. No se promete
ancho exacto de todos los glifos, aislamiento de procesos externos, deshacer
universal ni ausencia de carreras con otros programas que modifiquen el PATH.
Las operaciones de PATH revalidan el valor antes de escribir y comprueban el
resultado; un conflicto se informa y nunca justifica restaurar encima de cambios
ajenos. Pruebas de desregistro y borrado usan exclusivamente muestras descartables.

El registro persistente real se ejecutará al final mediante el instalador
revisado, tras los controles independientes. La desinstalación real del usuario
no forma parte de esta ejecución. Cada fallo, decisión, medición y veredicto se
conserva con fecha en `../memory.md` y en evidencias nuevas.
