# Mejoras de UI y comandos · 2026-10-29

Aprobado por el usuario tras revisión de mejora U01-U15 y cortes E1-E5.
Sin dependencias, paquetes, modelos ni cambios a PATH/Registro. Sin modificar
`ollama.model=null`, `num_ctx`, `num_predict`, `temperature`, `think`.
No se declara E6/v1 cerrada (P04 y P08 siguen pendientes).

## Alcance aprobado (C1 + C5 + C3)

- C1 `/help <comando>`: ayuda específica del comando con sus teclas
  reales, sin repetir todo el listado si se da un nombre.
- C5 Barra de reserva con porcentaje y color de aviso: añadir
  `porcentaje` y `color` a la línea `Reserva:` (ya existe
  `context_line`) sin alterarla si falta; solo visual.
- C3 Atajos Alt+P / Alt+E / Alt+T para cambiar entre modos
  Preguntar, Editar (chat) y Tarea sin escribir `/ask`/`/chat`/`/task`.

Lo que NO entra: tu propuesta 8 (`/gpu` de solo lectura), 6 (`/exit`
con confirmación), 7 (alias configurables), 4 (búsqueda en historial
`Ctrl+F`), ni P08 de pegado. Cada una requiere historia aparte.

## Diseño

- `/help` con argumentos: si `input.trim()` tras `/help` es non-empty y
  coincide con un nombre existente, muestra detalle del comando con sus
  teclas (ej. `/permissions: Enter detalle, Ctrl+Enter concede/retira,
  flechas marcan fila, Esc cierra`); si no, conserva el mensaje actual.
- Barra de reserva: `Reserva: usado/límite (X%)`; si X >= 80, color de
  aviso (amarillo/rojo conservador); si >= 95, rojo. Sin cambiar datos.
- Alt+P/E/T: eventos `Key::AltP`, `Key::AltE`, `Key::AltT` (o mapeo
  equivalente en `Key`) que invocan los mismos cambios que `/ask`,
  `/chat` y `/task`. No crean nuevos modos ni amplían permisos.

## Configuración validada

En `config/defaults.json`: `ui.reserve_percent_warning: 80`,
`ui.reserve_percent_critical: 95`, `ui.reserve_color_warning: true`.
Los valores predeterminados conservan la línea como hoy; los nuevos solo
actúan si el usuario los activa explícitamente.

## Comprobación

- Formato/Clippy/tests/release offline PASS.
- Tester independiente PASS sobre comandos, barra de reserva y atajos.
- Reviewer APPROVE sin hallazgos bloqueantes en las fuentes nuevas.
- Consola real 73x21 con el binario actualizado; documento de evidencia
  `evidencias/2026-10-29-mejoras-ui.json`.
- Bitácora (`memory.md`) y README actualizados con alcance real y límites.
