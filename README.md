# PillCLIterminal (`pill`)

Agente de programación **local** para la terminal de **Windows**, escrito en **Rust**.
Habla con **Ollama en tu propio equipo** (sin nube), edita tu proyecto con
herramientas reales y exige **permisos explícitos** para cualquier acción con
efectos. Se invoca como `pill` desde una consola interactiva ya abierta (PATH registrado en `HKCU:\Environment`; `D:\PROJECTS\PILLCLITER\bin` incluido). `pill --help` y `pill --version` confirman ejecución; `Ctrl+C` restaura la consola exacta.

Ejecutar sin reiniciar: si el PATH de sesión no incluye `bin`, usar `.\bin\pill.exe` directamente. `/gpu` ejecuta `nvidia-smi` en modo solo lectura (sin atribución de procesos, sin permisos, sin matar GPU).

[Guía de instalación y mejoras](INSTALACION-Y-MEJORAS.md) ·
[Bitácora completa](memory.md) · [Plan aprobado](PLAN-IMPLEMENTACION.md)

## Qué es

`pill` es una cápsula de chat + Tarea que vive en tu consola. No instala
modelos, no elige ninguno por ti (`ollama.model` sigue `null`) y no envía tu
código a Internet: toda la inferencia pasa por el Ollama de loopback
(`http://localhost:11434`, solo HTTP local, sin proxies ni redirecciones).
Cada capacidad de esta lista está verificada con pruebas e investigación
independientes; lo pendiente se declara pendiente.

Estado del corte más reciente: **2026-10-28 · tester PASS y reviewer APPROVE**
con release `62a1857654b2a15fc793029f2f54bc26e36e5ec815eb4e834d366180ae47f90f`.
[Tester](evidencias/2026-10-28-pulido-tester-final.json) ·
[Reviewer](evidencias/2026-10-28-review-polish.json).

## Características

### Interfaz nativa de Windows
- Consola Win32 con cápsula Unicode/ASCII configurable (`logo_color`: `green`, `cyan`, `white`).
- Exige consola interactiva: la entrada/salida redirigidas y la consola propia de un doble clic se rechazan.
- Menú `/` que se despliega desde el primer carácter, filtrable; Tab completa, flechas eligen, Esc cierra conservando la entrada.
- Entrada Unicode multilínea con edición por **grafemas completos** (familias ZWJ, marcas, selector de variación, banderas).
- Texto de razonamiento plegable con **Ctrl+R**; el historial guarda solo «Razonamiento plegado».
- **Ctrl+Enter envía**, Enter añade línea (también al pegar texto). Ctrl+C sale y restaura la consola exactamente.
- Paginación real en borrador e historial; PgUp y PgDn mueven una pantalla.
- Cabecera con contexto usado/límite, modelos de la sesión, estado de la tarea y segundos en curso.

### Chat local con Ollama
- Chat en streaming con cancelación y reintentos reales.
- Selección de modelo siempre explícita, nunca automática; `/model` muestra tamaño y si el modelo está en memoria.
- Validación del digest del modelo antes y después de generar; historial con una cota conservadora de bytes.
- Persistencia de la conversación en `.pillcli/` y recuperación solo si nombre y digest coinciden.
- Orden de liberación exclusiva confirmada antes de actuar (`/liberate`).

### Herramientas de proyecto con permisos
- Listar, buscar texto literal, leer UTF-8, crear sin sobrescribir y **editar una coincidencia exacta con SHA-256** de una lectura previa.
- Ediciones con respaldo en transacción; conflicto y resultado parcial declarados, nunca inventados.
- Lecturas reales en paralelo acotadas por `agents.max_parallel_reads`.
- Comandos del proyecto solo si **tú** los concedes en `/permissions`: sin shell implícito, con timeout, cancelación de su árbol de hijos y concesiones ligadas al programa, argumentos y contenido observado.
- Cambiar el script invalida la concesión. La configuración por sí sola no concede nada.
- Concesiones guardadas fuera del proyecto en `%LOCALAPPDATA%/PILLCLI/trust.json`.
- Servidores **MCP stdio** locales: esquema validado sin `$ref` externo, programa y archivos fijados por handles; muerte, timeout o desviación bloquea nuevos efectos sin presupuestar que no pasó nada.

### Tarea multiagente
- Cinco roles con historiales separados: coordinador, explorador, implementador, tester y reviewer.
- Handoff solo con **hechos de herramientas reales**: ruta, estado, hashes, código de salida. La prosa o los hashes inventados del modelo no cruzan el handoff.
- El cierre exige un comando real exitoso del tester, revalidación de archivos probados y veredicto JSON APPROVE del reviewer tras leer lo cambiado.
- Fallos y CHANGES_REQUESTED se reencolan, acotados por `agents.max_steps`; un modelo puede agotarse: la tarea queda pendiente y su efecto visible en el diario.
- Handoff entregado como hechos, no opiniones. Historiales separados y correlacionados por sesión.
- Diario `.pillcli/sessions/*.jsonl` con rol, hashes y resultados; `/journal` lo muestra sin crear nada.
- Reconciliación al arrancar entre diario y disco: aplicado, pendiente o desconocido, sin repetir efectos; lo desconocido mantiene bloqueadas las escrituras incluso tras reiniciar. `/recover` distingue revisadas y pendientes con un contador visible.
- `/archive` guarda la conversación sin destruirla y `/copy` copia al portapapeles.

### Sin éxitos ficticios
- Un rol que solo afirma haber actuado no cierra su rol: recibe retroalimentación acotada. Hay error tipado `EmptyResponse`.
- Sin stubs ni mocks en producción; límites declarados en `spec/03-PENDIENTES.md` y en la bitácora.

## Comandos

| Comando | Función |
|---|---|
| `/help` | Ayuda de la sesión |
| `/model` | Elegir modelo instalado, ver tamaño y estado en memoria |
| `/permissions` | Conceder o retirar comandos y servidores MCP, fila a fila |
| `/task` | Modo Tarea multiagente para la próxima petición (exige un comando concedido) |
| `/chat` | Vuelta al chat con herramientas (alias `/edit`) |
| `/status` | Modelo, configuración, recursos y cuotas |
| `/ask` | Preguntar con herramientas de solo lectura |
| `/edit` | Editar con herramientas y permisos vigentes |
| `/changes` | Inspeccionar cambios y resultados observados |
| `/journal` | Últimas líneas del diario actual |
| `/recover` | Revisión de recuperaciones con pendientes y revisadas |
| `/undo` | Restaurar un respaldo con versión comprobada |
| `/profile` | Elegir perfil de contexto y generación |
| `/archive` | Archivar evidencia revisada con confirmación |
| `/liberate` | Descarga de memoria de los modelos de esta sesión |
| `/copy` | Copia al portapapeles la respuesta copiable |
| `/exit` | Salir con restauración exacta de la consola |

## Instalación

Requiere Windows con consola interactiva, **Rust 1.94 estable con MSVC** y
**Ollama iniciado** con al menos un modelo que anuncie herramientas (`tools`).

```powershell
git clone https://github.com/zerumen82/PillCLIterminal.git
cd PillCLIterminal
cargo build --release --locked --offline
.\scripts\Install-Pill.ps1 -SkipBuild         # registra bin\ en el PATH de usuario
```

`Install-Pill.ps1` copia el binario compilado a `bin\` y añade únicamente esa
carpeta al PATH de usuario conservando las entradas existentes; no crea
accesos directos. `-SessionOnly` registra solo la sesión actual. Cambiar los
valores incluidos de `config/defaults.json` requiere recompilar; los detalles
están en la [guía de instalación](INSTALACION-Y-MEJORAS.md).

```powershell
pill                                   # en una nueva consola, con PATH registrado
pill --project D:\PROJECTS\algo        # raíz del proyecto (por defecto, la carpeta actual)
pill --config preferencias.json        # ruta relativa al proyecto
pill --model NOMBRE_INSTALADO          # elige el modelo explícitamente
pill --help
```

La primera vez no hay modelo configurado: se consultan los instalados y tú
eliges con número + Ctrl+Enter o `--model`. Nunca se descargan modelos ni se
elige el primero automáticamente.

Configuración opcional, en `.pillcli/config.json` o en el archivo de
`--config` (se combinan con los valores incluidos):

```json
{"ui":{"ascii_logo":true,"logo_color":"cyan"}}
{"ollama":{"num_ctx":4096,"num_predict":512,"temperature":0.2,"think":false}}
```

Los campos desconocidos y valores inválidos producen error; la configuración
nunca concede permisos.

## Desarrollo y comprobación

Requiere Rust 1.94, Clippy y rustfmt. No usar comandos de otros ecosistemas.

```powershell
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
cargo build --release --locked --offline
.\scripts\Install-Pill.ps1 -SkipBuild -SessionOnly
python diagnostics/test_console.py
```

Las pruebas reales con Ollama (chat, cancelación, errores), con herramientas y
con los cinco agentes son **opt-in** (`--ignored --exact`) y no cuentan como
PASS si no se ejecutan; los comandos exactos están en el README detallado de la
guía y en `tests/`. Perfil verificado por el tester en consola, solo como
muestra reproducible (no es elección de modelo principal):

```json
{"ollama":{"model":"huihui_ai/qwen3-abliterated:4b-v2","num_ctx":16384,"num_predict":2048,"think":true}}
```

El diagnóstico de consola Python usa solo biblioteca estándar: levanta
consolas Windows reales ocultas, inyecta eventos Win32 y comprueba menú,
Unicode, redimensionado y restauración exacta.

## Límites explícitos

- Una muestra de recursos observados no es una promesa general de RAM/CPU; el tamaño lógico de un modelo no es su VRAM; no se promete la representación de todos los glifos ni resistencia a cortes de energía.
- El criterio de consola interactiva no identifica quién invocó el proceso ni la antigüedad de la consola.
- No hay transporte MCP HTTP remoto ni sampling; el programa no elige el modelo principal.
- **E6/v1 no está cerrada**: quedan pendientes la elección del modelo principal (P04, sigue `null`) y la política adicional de pegado (P08), decisiones reservadas al usuario.
- El trabajo del corte 2026-10-29 (diff y separación visual) está en curso y **sin** tester PASS ni reviewer APPROVE aún; solo lo documentado en `spec/15-DIFF-CONVERSACION.md`.

## Estructura

```
src/main.rs src/lib.rs src/cli.rs src/launch.rs   arranque, opciones y control de consola interactiva
src/config/ y config/defaults.json                configuración validada; el modelo incluido es null
src/ui/                                           cápsula, entrada por grafemas, menú y consola Win32
src/providers/ollama/                             chat, streaming y errores sobre HTTP loopback
src/files/                                        lectura, búsqueda, creación y edición con SHA-256
src/tools/                                        herramientas, permisos, diario y cliente MCP stdio
src/gpu/                                          criterio de liberación compartida o exclusiva
src/agents/                                       coordinador, explorador, implementador, tester y reviewer
src/session/                                      trabajador de la sesión, avisos, /liberate y recuperación
tests/                                            pruebas reales y opt-in de Ollama, MCP, GPU y archivos
diagnostics/                                      sondas de consola y comprobaciones Python
spec/                                             especificaciones: mandan sobre el código
evidencias/                                       resultados fechados del tester y del reviewer
memory.md                                         bitácora completa del proyecto en español
```

## Plan y reglas

[Plan aprobado](PLAN-IMPLEMENTACION.md) · [Principios](spec/00-PRINCIPIOS.md) ·
[Requisitos](spec/02-REQUISITOS.md) · [Pendientes](spec/03-PENDIENTES.md) ·
[Bitácora](memory.md)

Esta documentación solo anuncia capacidades verificadas; lo pendiente queda
declarado pendiente. Interacciones en español, cómputo local primero y
dependencias de red siempre declaradas.
