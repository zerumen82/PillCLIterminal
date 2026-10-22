# Roles de agentes y flujo

Estado: aprobado en el plan, 2026-10-20. Distinguir los agentes que desarrollan PILLCLI de los agentes que tendrá el producto.

## Equipo de desarrollo

| Rol global | Responsabilidad |
|---|---|
| Coordinador | Leer reglas, especificaciones y memoria; pedir decisiones al usuario e integrar trabajo aprobado |
| Scout | Revisar skills disponibles y posibilidades de reutilización; no instalar nada sin aprobación |
| Investigador | Consultar fuentes primarias para APIs, dependencias y compatibilidad; registrar límites de lo investigado |
| Editor | Implementar historias aprobadas; no ampliar alcance ni dependencias |
| Reviewer | Revisar calidad, seguridad, permisos y mantenibilidad; APPROVE o CHANGES_REQUESTED |
| Tester | Ejecutar criterios sobre la versión real; PASS o FAIL, con evidencia |

Se utilizará la delegación disponible para revisión independiente. No es necesario crear seis procesos o agentes a la vez; el coordinador puede cubrir investigación y edición. Los veredictos deben mantener independencia de la implementación cuando el entorno lo permita.

El catálogo de skills ya contiene `plan`, aplicada a esta fase. Buscar o evaluar no autoriza instalar skills; no ejecutar mecanismos de descubrimiento que descarguen paquetes sin autorización.

## Agentes del producto

| Rol | Capacidades propuestas |
|---|---|
| Coordinador | Descomponer, asignar, esperar dependencias, integrar y responder con evidencia |
| Explorador | Listar, buscar y leer; devolver rutas y versiones observadas |
| Implementador | Preparar cambios con base; pedir aplicación al ejecutor central autorizado |
| Tester | Solicitar comprobaciones registradas y asociar resultados a versión concreta |
| Reviewer | Leer propuestas/diffs/evidencia; emitir observaciones sin aplicar cambios por su cuenta |

Todos pueden compartir el mismo modelo local. Cada uno conserva conversación y herramientas separadas. El coordinador limita pasos y concurrencia; no confundir varias funciones en un único prompt con delegación ya implementada.

## Flujo de desarrollo

1. Usuario aprueba historia, diseño y dependencias necesarias.
2. Editor implementa una etapa funcional; sin éxitos vacíos ni componentes ficticios para aparentar el flujo completo.
3. Reviewer inspecciona la versión exacta y devuelve veredicto.
4. Tester ejecuta los criterios correspondientes sobre esa versión.
5. Ante cambios, repetir los controles afectados; una aprobación antigua no cubre código nuevo.
6. Actualizar requisitos, pendientes y `memory.md` con evidencia. Solo entonces declarar terminada la etapa, identificando qué queda fuera.

## Coordinación del producto

Las subtareas independientes de lectura pueden avanzar en paralelo con límite configurable. Las inferencias empiezan serializadas y los cambios se aplican por el ejecutor central. El coordinador rechaza bases obsoletas, no decide que la última escritura gana.

Errores, cancelaciones y resultados parciales se propagan con identificadores de sesión/tarea/agente/llamada. Ningún hijo amplía raíces, permisos, herramientas o límites por citar una orden del coordinador. Comandos y MCP con efectos participan en la misma coordinación de modificaciones.

La respuesta final diferencia propuesta, aplicación y verificación. El estado «no probado» no se transforma en PASS por acuerdo entre agentes.
