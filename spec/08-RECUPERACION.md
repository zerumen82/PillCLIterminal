# R08/R14 · Reconciliación local del diario

Diseño aprobado por el usuario el 2026-10-27: contrastar el diario con el disco al arrancar, sin repetir escrituras ni restaurar sobre cambios posteriores. No añade dependencias.

## Contrato

- La composición de confianza lee `.pillcli/sessions/*.jsonl` por handles de directorio fijados; no abre rutas tomadas del texto del diario fuera de las herramientas `Files`. Conserva los límites `agents.max_sessions`, `agents.max_journal_bytes` y `agents.max_record_bytes`.
- Cada `requested` se correlaciona con su `observed` mediante `call_id` dentro de su diario. Los registros incompletos, inválidos, excesivos o contradictorios dejan resultado `unknown`; no se ignoran para habilitar efectos.
- Las herramientas sin efectos no necesitan reconciliación. Un comando o MCP solicitado sin cierre observado deja `unknown`: el estado de un archivo no prueba todos sus efectos.
- Para archivos, se comparan únicamente las rutas relativas autorizadas y los SHA-256 registrados. Un archivo creado/editado con hash objetivo presente tiene estado `applied`; una base de edición intacta o un destino de creación aún ausente tiene estado `pending`. Cualquier otra combinación sin cierre es `unknown`.
- Mover se contrasta por origen y destino, sin sobrescritura: origen ausente y destino con hash previsto es `applied`; origen con base intacta y destino ausente es `pending`; el resto es `unknown`. Borrado sin cierre es `applied` si se verifica ausencia, `pending` si la base sigue intacta y `unknown` en otro caso. Las carpetas vacías usan la inspección/versionado de R09.
- Un resultado duradero `partial`, `unknown` o `applied` conserva el bloqueo aun si una ruta coincide después. La coincidencia no demuestra que todos los efectos o respaldos se hayan reconciliado.
- Un `verified` duradero acredita la observación histórica. Se informa si el archivo cambió después, pero no se convierte una edición posterior del usuario en una operación incierta anterior.
- El informe identifica diario, llamada, operación y cada ruta de forma separada. Un lote conserva los resultados observados de los archivos completados y los estados de las llamadas pendientes; no promete atomicidad global.
- `needs_review` bloquea nuevos efectos tras reinicio. Cambiar de modelo o volver a cargar el historial no lo levanta. El informe no reproduce comandos, MCP, movimientos, borrados ni escrituras.

## Integración y aceptación

`tools::recovery::reconcile(&Files, max_sessions, max_journal_bytes, max_record_bytes)` devuelve un informe serializable con `entries`, `needs_review` y `summary()`. La sesión muestra ese resumen y el ejecutor conserva el bloqueo antes de admitir efectos. La prueba usa diarios y archivos temporales reales: creación/edición aplicada sin respuesta, base intacta, cambio externo, comando/MCP inconcluso, diario truncado, resultado parcial tras reinicio y lote interrumpido. Deben conservarse todos los bytes y no aparecer efectos nuevos al reconciliar.

El cierre requiere las comprobaciones del proyecto, tester PASS y reviewer APPROVE sobre las fuentes finales; esta especificación no declara esas verificaciones realizadas.
