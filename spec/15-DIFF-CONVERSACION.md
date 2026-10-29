# Diff y conversación legibles · 2026-10-29

El usuario aprobó «hazlo» tras recomendar empezar por diff y separación visual.
Se aplica plan, con AGENTS global/local y memory como fuentes. Alcance: las
mejoras1y2 de esa propuesta; cápsula, backend, permisos, modelos y PATH se
conservan. Sin paquetes nuevos, commits ni desinstalación. El menú agrupado y
los detalles con indicador de posición quedan para otra entrega.

## Presentación

- Detalle de cambios con diff de líneas: prefijos ASCII +/-, contexto y
  cabeceras de rango @@. Colores opcionales verde/rojo; el texto basta sin color.
- Comparación literal de líneas incluyendo sus terminadores, con indicación
  de CRLF/LF/sin salto final. Contexto sobre la comparación de contenido; no
  normalizar archivos ni crear una herramienta de aplicación de parches.
- LCS acotado por celdas tras quitar prefijo/sufijo común; si excede presupuesto,
  agrupar el bloque modificado y declarar el límite. Sin comparación cuadrática
  ilimitada, red ni dependencias. No afirmar minimalidad del fallback.
- Metadata de estado, ruta, SHA y respaldo antes del diff. Conservar causas de
  fallo y estados desconocidos. La vista no convierte propuesta en cambio aplicado.
- El color se añade después de sanear/envolver texto, desde tipos propios.
  Ninguna secuencia de escape del contenido se trata como formato confiable.
- Nuevos valores validados en defaults: ui.color=true,
  ui.diff_context_lines=3 (0..16), ui.diff_max_cells=262144 (1..4194304).
  ui.max_history_bytes acota la vista; indicar recortes y evitar calcular diff
  si las dos fuentes ya exceden ese límite. Nada de esto modifica datos fuente.
- Mensajes separados por encabezados de texto «Tú», «Respuesta» y «Acción»;
  respuesta en curso identificada. Texto persistido compatible con lines
  existentes; sin reinterpretar respuestas antiguas ni cambiar el transcript
  del modelo. Los encabezados son presentación, no concesiones ni evidencia.

## Implementación y verificación

Módulo puro src/ui/diff.rs; integración en model/render; defaults/config
validados. Conservar caché Arc y selección visible del corte14. Probar diff
con varios hunks, repetidos, vacío, creación, CRLF/BOM, sin newline, límite,
Unicode/controles; color/no color, cursor y paginación a73x21. Backend/hashes
sin cambios. Verificar orden de conversación, streaming y recuperación local.

Formato/Clippy/tests/release offline, reviewer APPROVE y tester PASS sobre
fuentes finales. Consola real con muestras descartables y metadatos locales;
inferencia explícita solo si aporta cobertura necesaria. Actualizar bin
instalado por SessionOnly tras controles, conservando propiedad y PATH de usuario.
Anotar cada fallo/resultado en memory.md y evidencias nuevas.
