# Persistencia, ediciones y recuperación guiada · 2026-10-27

Diseño autorizado por «arréglalo todo» y `10-USABILIDAD-FIABILIDAD-INSTALACION.md`.
No añade dependencias ni cambia concesiones desde herramientas del modelo.

- Archivos privados: preparar bytes completos y sincronizados en transacción
  bajo el directorio privado fijado. Mover la copia anterior a respaldo por su
  handle y publicar sin reemplazo; una colisión conserva los bytes ajenos.
  Nunca truncar la única copia válida. La ventana sin nombre es explícita y
  la lectura puede recuperar la base de una transacción validada. Se rechazan
  hardlinks y reparse antes de efectos. Cuota de versiones: `files.max_transactions`.
  Los metadatos de recuperación `base-info.json` se escriben, sincronizan y
  verifican una sola vez antes de mover la base, con sobre SHA-256 propio.
  El manifiesto de estado puede quedar truncado sin anular ese respaldo.
  Archivo de una versión privada exige revalidar manifiesto, sobre, base,
  identidad y miembros tanto antes como después del movimiento; un fallo
  posterior se declara parcial y conserva los bytes.
- Historial de modelo: formato versionado, sistema vigente al restaurar y
  validación de roles/relación entre assistant con llamadas y resultados tool.
  Recortar turnos completos, sin resultados huérfanos. Un error o recuperación
  de respaldo se muestra; no se afirma guardado exitoso al descartar un error.
- Ediciones múltiples: una versión SHA de base y varios reemplazos literales
  únicos, todos validados antes de publicar. Una ambigüedad rechaza todo el
  archivo. Una transacción y un respaldo por publicación.
- Restaurar respaldo: ruta dentro de la transacción propia, manifiesto y hashes
  verificados, destino con SHA posterior esperado. Conservar la versión actual
  en otro respaldo. Nunca restaurar sobre modificaciones posteriores ni
  presentar la restauración como deshacer universal.
- Recuperación: mostrar entradas y snapshots observados. Una confirmación
  humana se liga a SHA del diario y snapshots actuales; un cambio invalida la
  huella. El reconocimiento durable mantiene `unknown` como estado histórico,
  señalando revisión humana; no lo transforma en resultado de herramienta
  verificado ni reproduce efectos. Entradas no revisadas siguen bloqueadas.
- Archivo de sesiones: solo diarios concretos cuyo informe no requiere revisión,
  versión y huella revalidadas, movimiento sin reemplazo dentro del área privada.
  No borrar sesiones desconocidas ni respaldos para eludir cuotas.

Acciones humanas acordadas con el coordinador: Recovery,
AcknowledgeRecovery{fingerprint}, Changes y Undo con ruta, SHA actual y respaldo.
La UI confirma mediante Ctrl+Enter. Ninguna de estas capacidades se concede al
modelo por texto o por editar configuración. Las pruebas usan disco real y
muestras descartables, incluyendo colisión, fallo de publicación, historial
inválido, BOM/CRLF, base modificada y reconocimiento obsoleto.

Implementación de esta parte preparada el 2026-10-28: pruebas de disco ampliadas
en `tests/persistence_real.rs`, además de los casos anteriores y las regresiones
de historial en la sesión. `rustfmt` y revisión de espacios PASS; Cargo y la
verificación independiente se serializan por el coordinador antes del cierre.
La inyección de corrupción representa estados de corte; no se afirma una
prueba de corte físico de energía ni garantía frente a daños arbitrarios del disco.

La retencion incluye versiones privadas y respaldos de edicion verificados.
Los respaldos de proyecto se mueven a un localizador SHA de su ruta original;
una restauracion sigue resolviendo la referencia original y valida manifiesto,
base y SHA posterior. Las cuotas/exploraciones que no se pueden enumerar se
exponen como desconocidas sin bloquear otras ofertas completas y seguras.
El SHA previsto y la publicacion comparten transformacion sobre base unica:
los parches usan offsets originales, se rechazan solapes entre parches y
coincidencias literales solapadas dentro de un mismo `old`.
