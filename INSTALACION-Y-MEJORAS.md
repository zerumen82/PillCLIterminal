# pill: instalación y mejoras propuestas

Guía para este proyecto en `D:\PROJECTS\PILLCLITER`. Actualizada el 22 de octubre de 2026.

## Qué funciona ahora

Terminal verificada: cápsula, interfaz en español, configuración JSON y menú automático al escribir `/`. Comandos disponibles: `/help`, `/exit`, `/model`, `/permissions`, `/task`, `/chat` y `/liberate`; Ctrl+C también permite salir.

E2 incorpora chat con Ollama, selección explícita, respuesta progresiva y cancelación. E3/E4 permite leer/buscar/crear/editar archivos del proyecto, conceder comandos registrados y completar una corrección con cinco roles y pruebas reales, con tester PASS y reviewer APPROVE sobre el binario de aquel corte. El [perfil reproducible](README.md#herramientas-y-tareas) usa 4b-v2 con contexto 16384, razonamiento activado y generación máxima 2048; los valores incluidos no se cambian automáticamente. Desde el 2026-10-22 hay cliente MCP stdio y `/liberate`, comprobados en local y todavía sin tester ni reviewer independientes. Lecturas paralelas, mover/eliminar y recuperación automática siguen pendientes. No se declara v1.

## Instalación con la compilación existente

Abre PowerShell en una consola y ejecuta:

```powershell
cd D:\PROJECTS\PILLCLITER
.\scripts\Install-Pill.ps1 -SkipBuild
Get-Command pill
pill --version
pill
```

`Get-Command pill` debe apuntar a `D:\PROJECTS\PILLCLITER\bin\pill.exe`. La versión actual es `pill 0.1.0`.

El script necesita `target\release\pill.exe`: lo copia a `bin\pill.exe` y añade ese directorio al PATH de usuario y de la sesión. Conserva las entradas existentes y detecta conflictos con otro comando llamado `pill`. Este paso no descarga paquetes ni modelos. No necesitas Rust para registrar una compilación ya existente.

Después podrás invocarlo como `pill` desde cualquier carpeta cuyo proceso de consola tenga ese PATH. El proyecto activo será la carpeta desde la que lo lances. También puedes indicarlo explícitamente:

```powershell
pill --project D:\PROJECTS\PILLCLITER
```

Mantén el directorio del proyecto en esa ubicación: el comando registrado depende de su carpeta `bin`.

## Si no se puede guardar el PATH

El entorno de desarrollo rechazó la escritura del Registro; **el registro permanente no está confirmado**. Si ocurre en tu consola, usa la opción para esa sesión:

```powershell
cd D:\PROJECTS\PILLCLITER
.\scripts\Install-Pill.ps1 -SkipBuild -SessionOnly
pill
```

Si una consola nueva no reconoce `pill`, puedes añadir la carpeta solo a su sesión:

```powershell
$env:Path += ';D:\PROJECTS\PILLCLITER\bin'
Get-Command pill
pill
```

Las consolas abiertas conservan su propio entorno; guardar el PATH de usuario no actualiza automáticamente todos los procesos existentes. Véase [entorno de procesos en Windows](https://learn.microsoft.com/en-us/windows/win32/procthread/environment-variables).

## Si falta el binario o quieres recompilar

Para compilar necesitas Rust/Cargo 1.94 o superior compatible y herramientas C++ de MSVC. Esta máquina ya compiló el proyecto; la guía no instala herramientas del sistema.

```powershell
cd D:\PROJECTS\PILLCLITER
.\scripts\Install-Pill.ps1
```

Sin `-SkipBuild`, el script ejecuta `cargo build --release --locked --offline` antes de registrar el comando. En otra máquina, la compilación offline requiere tener las dependencias en caché. Si faltan, habrá que autorizar su descarga; no basta con copiar este documento.

## Uso y actualización

Escribe `/` para desplegar el menú, flechas para seleccionar, Tab para completar, Enter para ejecutar y Esc para cerrar conservando la entrada. La interfaz necesita entrada/salida interactivas; ayuda y versión pueden consultarse sin abrirla.

Con Ollama iniciado, elige un número de la lista local y pulsa Ctrl+Enter, o inicia con `pill --model NOMBRE_INSTALADO`. **Ctrl+Enter envía peticiones; Enter añade línea.** Esc cancela la operación activa. Ctrl+R abre el razonamiento plegado. `/journal` lee el diario local. Mientras un trabajo sigue, la barra muestra los segundos. La primera elección del mismo modelo recupera la conversación guardada; `/model` más adelante inicia otra. No hay selección automática ni descargas implícitas; los modelos sin herramientas anunciadas permiten solo chat de texto. Nombre, digest y métricas reales aparecen en la interfaz.

Para actualizar tras cambiar el código, sal de las sesiones de `pill` y vuelve a ejecutar `scripts\Install-Pill.ps1` desde el proyecto. `-SkipBuild` copia la última compilación existente, aunque el código fuente haya cambiado.

Configuración opcional en `.pillcli/config.json`:

```json
{"ui":{"ascii_logo":true,"logo_color":"cyan"}}
```

Los colores admitidos son `green`, `cyan` y `white`. Las demás preferencias están en [config/defaults.json](config/defaults.json). Los valores incluidos se integran en el binario al compilar; editarlos requiere recompilar. La configuración de proyecto se lee al iniciar.

E3/E4 incorpora herramientas de archivos y comandos registrados. El chat puede crear/editar dentro del proyecto con base SHA-256; el ejecutor protege su instalación y las áreas de control. `/permissions` muestra comandos, servidores MCP, raíces extra y exclusividad: las flechas marcan una fila, Enter muestra el detalle y **Ctrl+Enter** concede o retira; Esc cierra. Una raíz extra no se concede ni se retira. La configuración no concede permisos por sí sola. `/task` activa roles separados para una tarea con pruebas concedidas; `/chat` vuelve al chat con herramientas. Consulta [contratos, configuración y límites](README.md#herramientas-y-tareas), incluidos respaldos persistentes, efectos parciales y contexto necesario. Un servidor MCP se propone en `mcp.servers` y solo arranca después de concederlo. `/liberate` y el arranque descargan únicamente modelos que esta sesión seleccionó, y solo si la exclusividad está concedida.

## Mis sugerencias de mejora

Son recomendaciones para que elijas; este documento no las implementa ni cambia las prioridades aprobadas.

### Completar el plan ya aprobado

| Propuesta | Resultado que conviene exigir |
|---|---|
| Elegir modelo principal con evidencia | Chat y tres muestras E2 verificados; un perfil de cinco roles E3/E4 pasó en consola. Consultar [perfil y límites actuales](README.md#herramientas-y-tareas); elegir principal sigue pendiente del usuario. |
| Ampliar la cobertura de herramientas y coordinación | Base nativa y muestra de corrección verificadas. Quedan lecturas paralelas, mover/eliminar y las integraciones indicadas en requisitos. Una afirmación del modelo no prueba una escritura. |
| Comparar los tres modelos instalados | Medir calidad en tareas de código, latencia, RAM/VRAM y reparto CPU/GPU con el mismo contexto; elegir con datos propios. |
| Cerrar E5 con tester y reviewer, y medir la GPU real solo si se autoriza | La liberación local ya conserva lo ajeno y descarga solo lo seleccionado en un servidor de prueba. No se vació la GPU del usuario. Ollama documenta `keep_alive`, pero `/api/ps` no identifica al cliente. [Referencia oficial](https://docs.ollama.com/faq). |

### Mejoras adicionales que propondría

- **Unicode y protección adicional:** en el chat, Enter añade línea y Ctrl+Enter envía. En permisos, Enter muestra el detalle y Ctrl+Enter concede o retira; los comandos del menú siguen usando Enter. Retroceso, Supr e izquierda/derecha borran o mueven un grafema local. El ancho en pantalla sigue siendo conservador y una política adicional de pegado del menú sigue pendiente.
- **Instalación más sólida:** comprobar el comando después de registrarlo, notificar cambios de entorno a Windows y ofrecer actualización y desinstalación controladas.
- **Diagnóstico visible:** proponer `/doctor` y `/status` para mostrar configuración efectiva, conectividad, modelo y actividad. Estos comandos aún no existen.
- **Diferencias y trazabilidad claras:** presentar qué archivo cambiará, qué se aplicó y qué pruebas se ejecutaron; conservar resultados y fallos asociados a cada tarea sin registrar secretos.

La continuación E3/E4 comprueba tareas de lectura/edición con pruebas reales. Las mejoras adicionales requieren tu decisión.

Detalles y pruebas existentes: [README](README.md), [plan](PLAN-IMPLEMENTACION.md), [pendientes](spec/03-PENDIENTES.md), [tester E2](evidencias/2026-10-20-e2-tester.md) y [revisión E2](evidencias/2026-10-20-e2-review.md).
