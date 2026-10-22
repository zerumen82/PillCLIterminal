# Principios y definición de listo

Estado: plan y dependencias aprobados por el usuario el 2026-10-20, con la precisión de invocación `pill` exclusivamente desde consola. Base E1 e integración E2 verificadas con tester PASS y reviewer APPROVE; avance E3/E4 registrado a continuación. E5 está implementada y comprobada en local el 2026-10-22; su cierre independiente y E6 siguen pendientes. La existencia de especificaciones no acredita funcionalidades sin evidencia.

Continuación E3/E4: herramientas de archivos, comandos concedidos y cinco roles implementados; formato/lint/48 pruebas/build offline PASS. Tester independiente PASS (26/26 comprobaciones de consola) y reviewer de código APPROVE sobre el binario de aquel corte, con perfil explícito de razonamiento documentado. Los fallos de modelo observados se conservan; el ejecutor rechaza falsos éxitos y aplica feedback/reintentos acotados y traspasos de hechos reales. Lecturas paralelas, mover/eliminar y recuperación automática conservan sus pendientes. MCP y `/liberate` pasaron a E5; aquel PASS no los cubre.

## Decisiones aprobadas por el usuario

- Aplicación de terminal para Windows, desarrollada en Rust, con logo propio de cápsula.
- Nombre de invocación: `pill` (`pill.exe` en Windows). Requiere una consola interactiva existente; no ofrece inicio gráfico, acceso directo ni doble clic como flujo de arranque.
- Ollama inicialmente, modelos intercambiables y variantes publicadas como `uncensored`/`abliterated`.
- Coordinador multiagente y cliente MCP local `stdio` en v1.
- Lectura y escritura sin confirmaciones por archivo. Pruebas y compilación dentro del proyecto permitidas; instalaciones y operaciones destructivas requieren confirmación.
- Menú automático al escribir `/`; comando exacto `/liberate` y liberación al iniciar, preservando trabajo activo y procesos ajenos.
- Interacciones y `memory.md` en español. Búsqueda web después de validar el funcionamiento local.

## Reglas de trabajo

Rigen [AGENTS.md](../AGENTS.md) y `C:\Users\INdaHouse\.agents\AGENTS.md`. El usuario decide alcance, diseño, dependencias y prioridades. No instalar paquetes ni modelos, realizar borrados o crear commits sin autorización explícita correspondiente.

1. NO HARDCODE: proveedor, modelo, rutas, límites, política y parámetros se concentran en configuración validada. Las opciones sugeridas deben ser visibles y modificables.
2. NO STUB: entregar capacidades completas por etapas. Una capacidad ausente se indica como no disponible y no figura como éxito.
3. NO FAKE: texto del modelo no acredita operaciones. Los estados de archivos, comandos y servidores proceden del ejecutor y de comprobaciones reales.
4. LOCAL-FIRST: inferencia y registros locales. Red de desarrollo para dependencias declarada; herramientas web y MCP remoto quedan fuera de la primera entrega.
5. BITÁCORA: registrar fecha, autorización, decisiones, comandos, resultados, fallos y evidencia en [memory.md](../memory.md).

## Límites de confianza

Los permisos del usuario se comprueban en el programa; ningún modelo, archivo leído, resultado MCP o agente hijo puede ampliarlos. Permitir lectura/escritura no promete acceso que Windows deniegue. Configuración del ejecutor y concesiones de permisos no se modifican mediante herramientas del modelo.

Propuesta para aprobación: proyecto activo como raíz inicial, ampliable por decisión explícita del usuario. Dentro de esas raíces, lectura/creación/edición no preguntan de nuevo. La aplicación debe mostrar el alcance efectivo, no sustituir silenciosamente la autorización por restricciones ocultas.

La v1 no es un sandbox de Windows: scripts y servidores externos son código con los privilegios del usuario. Directorio de trabajo y validación de herramientas no garantizan aislamiento de sus efectos.

## Definición de listo

- Requisito aprobado y criterio de aceptación cumplido sobre la versión entregada.
- Formato, lint, pruebas y compilación en verde; no reutilizar resultados invalidados por cambios posteriores.
- Verificación real en terminal y Ollama; herramientas de archivos observadas en disco y comandos con salida/código reales.
- Tester: PASS; reviewer: APPROVE. Una revisión documental no acredita funcionamiento.
- Evidencia reproducible y riesgos conocidos anotados en `memory.md` y [03-PENDIENTES.md](03-PENDIENTES.md).

Cuando todavía no exista manifiesto o código, los controles de aplicación son «no disponibles», nunca PASS por omisión.
