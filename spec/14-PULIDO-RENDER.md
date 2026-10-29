# Pulido de recuperación y render · 2026-10-28

El usuario aprobó «hazlo» tras recomendar empezar por las mejoras1y2:
recuperación legible/atajo de perfiles y evitar copiar todo el historial por
frame. Se aplican fix/refactor y las reglas globales/locales. No se amplían
permisos, dependencias, modelos ni el PATH. Se conservan las decisiones de
memory.md y la evidencia anterior; ninguna prueba antigua valida estas fuentes.

## Cambio1: claridad

- Filas de /recover indican pendiente, revisada o sin revisión pendiente,
  conservando el estado histórico (incluido unknown) y las huellas originales.
- Contador visible de pendientes/revisadas/total en recuperación y su detalle;
  no afirmar cero pendientes si el informe es desconocido/incompleto.
- Detalle de perfiles anuncia Ctrl+Enter aplicar perfil, que ya existe.
- Sin modificar reconciliación, reconocimiento durable ni concesiones.

## Cambio2: refactor de render

- Compartir por Arc la caché de líneas envueltas ya existente. Enlazar sus
  referencias con las líneas de respuesta y copiar solo el intervalo visible.
- Conservar exactamente contenido, cursor, scroll, indicadores, resize y límites.
  El detalle conserva su ajuste y orden propios. No alterar configuración.
- Paso separado tras regresiones de claridad en verde. Comparar selección de
  líneas contra el algoritmo anterior con historias/respuestas largas y scroll.

## Comprobación

Formato, Clippy, pruebas aplicables, release offline, reviewer independiente y
tester independiente. Consola real de recuperación individual y reinicio,
perfiles y 73x21. Medición repetida del camino de selección con historial largo:
CPU/tiempo, bytes de cadenas copiados y contexto exacto. La comparación por
proceso es observación y no promete ahorro universal ni aceleración de Ollama.
No se añade prueba vacía ni umbral temporal frágil. Tras PASS/APPROVE actualizar
el binario instalado mediante el instalador revisado, preservando propiedad del
PATH existente. Sin desinstalación, commits ni nuevos paquetes.

El alcance de esta entrega son1y2. El portapapeles aislado y la especificación
adicional de pegado P08 siguen fuera de esta entrega y pendientes de concretar.


## Resultado ejecutado

39pruebasUI trasclaridad PASS,41trasrefactor PASS y benchmark opt-in ejecutado
PASS. Controlesindependientes finales: formato/Clippy,162tests locales y
release offline PASS;9opt-in ignoradas en suitegeneral no cuentan comoPASS.
La medición de ventanas se ejecutó aparte en5muestras. Consola ampliada21/21
PASS en3hosts:73x21real, ACKindividual2→1→0, unknown conservado y reinicio.
DosFAIL de fixture de resize se conservaron antes de usar el HWND propio;
no exigieron cambiar producción. Baseline96968d falló el nuevo atajo como
regresión roja esperada. Reviewer APPROVE sobre las dos fuentesUI cambiadas.

Nueva versión62a185...47f90f instalada mediante SessionOnly con8checksPASS:
User/MachinePATH, propiedadUser e install_id conservados; hash/versión correctos.
Consola instalada PASS. Evidencia: `../evidencias/2026-10-28-polish-installed.json`
y `../evidencias/2026-10-28-polish-installed-console.json`.
