# pill

Agente local de programación para terminal Windows. Nombre de invocación aprobado: **`pill`**.

[Guía de instalación y sugerencias de mejora](INSTALACION-Y-MEJORAS.md).

## Estado real

Herramientas nativas E3/E4 y muestra de corrección con cinco roles verificadas en consola con **tester PASS y reviewer APPROVE**, además de la base E1/E2. [Evidencia y límites del tester](evidencias/2026-10-20-e34-tester.md). Ese cierre no cubre el código posterior. El 2026-10-22 el cliente MCP local, `/liberate` y la consola quedaron implementados en local. Además de la reserva, los permisos y el borrador, la interfaz pliega el razonamiento y lo abre con Ctrl+R, marca listas con flechas, resume cada herramienta, nombra el rol de la tarea, señala el diario, lo lee con `/journal`, cuenta los segundos de un trabajo, borra un grafema entero y confirma `/liberate` cuando hay modelos de esta sesión. Al salir guarda la conversación en el proyecto: el modelo solo la retoma si eliges el mismo nombre y digest. Formato, Clippy, 83 pruebas y build offline en verde. Release y `bin/pill.exe` comparten SHA-256 `FDD485C66C146517B3780FB65B065E646F38F345FE58A43259F7A3BEE3583127`. Faltan tester y reviewer independientes. No es el cierre de v1. Interfaz nativa Windows en Rust, cápsula Unicode o ASCII, entrada, configuración JSON y menú automático al escribir `/`. Comandos: `/help`, `/exit`, `/model`, `/permissions`, `/task`, `/chat`, `/journal` y `/liberate`. Lecturas paralelas, mover/eliminar y la reconciliación automática de efectos parciales siguen pendientes. El registro persistente del comando sigue pendiente, como se explica abajo.

El control compartido en `src/launch.rs` exige entrada/salida interactivas y una consola Windows con más de un proceso asociado. La consola propia con un único proceso del sondeo equivalente a doble clic se rechaza. Este criterio no identifica al invocador ni demuestra la antigüedad de la consola. El programa no crea una interfaz gráfica ni un acceso directo.

## Invocar desde la consola

El binario está en `bin/pill.exe`. Desde PowerShell, en esta carpeta, registra el comando:

```powershell
.\scripts\Install-Pill.ps1 -SkipBuild
pill
```

El script copia el binario compilado y añade únicamente `bin/` al PATH de usuario, conservando las entradas existentes. No crea accesos directos. Este entorno bloqueó la escritura del Registro: **el PATH persistente no está registrado**. Ejecuta el script en tu consola habitual; `-SessionOnly` permite registrar solo la sesión actual. Otras consolas deben heredar el PATH actualizado. Sin `-SkipBuild`, compila primero sin descargas.

```powershell
pill --project D:\PROJECTS\PILLCLITER
pill --config preferencias.json
pill --model NOMBRE_INSTALADO
pill --help
```

El proyecto predeterminado es la carpeta actual. Las rutas relativas de configuración parten del proyecto. Se combinan los valores incluidos con `.pillcli/config.json` o el archivo explícito:

```json
{"ui":{"ascii_logo":true,"logo_color":"cyan"}}
```

`logo_color` admite `green`, `cyan` y `white`. Los demás límites están en `config/defaults.json`; campos desconocidos o límites inválidos producen error. La configuración no concede permisos.

Al escribir `/`, usa flechas para seleccionar, Tab para completar, Enter para ejecutar y Esc para cerrar conservando la entrada. En las peticiones, **Ctrl+Enter envía y Enter añade línea**, también al pegar texto. Los comandos del menú conservan Enter para ejecutar. Ctrl+C sale y restaura la consola. El chat puede leer/crear/editar archivos del proyecto y ejecutar únicamente comandos registrados y concedidos.

## Conversar con Ollama

Ollama debe estar iniciado. Sin modelo configurado se consulta la lista instalada; elige un número y pulsa Ctrl+Enter. También puedes usar `--model` o `ollama.model` en tu configuración. Nunca se elige automáticamente el primer modelo ni se descargan modelos. `/model` consulta de nuevo. La primera elección de un arranque recupera la conversación guardada solo si el nombre y el digest coinciden; si no, no borra ese archivo. Una elección posterior inicia otra conversación y el siguiente arranque ya no revive la anterior.

La cabecera muestra nombre, digest, modo Chat o `Tarea · rol · paso`, la reserva (`Reserva: usado/límite`, la misma cuenta de bytes que impide enviar un historial demasiado grande; al pasar del 80 % avisa «cerca del límite») y los modelos que esta sesión seleccionó, junto con si la exclusividad está concedida. `/model` muestra el tamaño y si el modelo está en memoria. En esa lista, y en la de permisos, las flechas marcan la fila y Ctrl+Enter confirma; un número escrito sigue valiendo. Con el razonamiento activo, la barra cuenta caracteres y el historial guarda solo «Razonamiento plegado». Ctrl+R abre y cierra ese texto; no se mezcla con la respuesta. Mientras hay un trabajo, la misma barra añade los segundos transcurridos sin sustituir la actividad. El borrador puede ocupar varias líneas y el cursor queda en la línea real: Inicio y Fin se quedan en esa línea, y Ctrl+flechas saltan de palabra. Retroceso, Supr e izquierda/derecha borran o mueven un grafema entero: familia unida por ZWJ, marcas, selector de variación, tono, tecla o bandera de dos indicadores. En una sola línea, arriba y abajo recuperan peticiones, salvo con un menú o una lista abiertos. PgUp y PgDn mueven una pantalla. Si queda texto más antiguo, aparece «hay texto por encima»; si dejaste líneas nuevas fuera, aparece «hay texto por debajo». Escribir no devuelve el scroll al final y PgUp no pasa del texto más antiguo. Esc cancela solo la operación activa. `/liberate` pide Enter cuando esta sesión tiene modelos y la exclusividad está concedida; si no hay nada de esta sesión que descargar, actúa enseguida. Las respuestas parciales se identifican y no se incorporan al historial del modelo. Al salir, la consola guarda sus líneas en `.pillcli/console.json` y el trabajador guarda los mensajes que el modelo vio en `.pillcli/model.json`. Al volver, esas líneas se muestran como historial local. El modelo solo las recibe de nuevo si eliges el mismo modelo; si el archivo no cupo en `agents.max_journal_bytes`, se dice cuántos mensajes antiguos quedaron fuera. Un historial que supera el contexto configurado sigue rechazándose, sin recorte silencioso para continuar. La interfaz sigue aceptando entrada y redimensionado durante la generación. Tras una respuesta se muestran tiempos y, cuando el servidor los devuelve, tokens de entrada/generados y tiempo de carga. Un efecto parcial deja «efectos parciales · revisa» y la ruta corta del diario hasta cerrar el programa: `/model` inicia otra conversación y no vuelve a permitir escrituras.

```json
{"ollama":{"url":"http://localhost:11434","model":null,"num_ctx":4096,"num_predict":512,"temperature":0.2,"think":false}}
```

Se admite solo HTTP loopback, sin proxies ni redirecciones. Contexto, generación, tiempos y tamaños están en `config/defaults.json`; se valida el contexto contra lo anunciado por el modelo y se comprueba su digest antes y después de generar. El historial tiene una cota conservadora de bytes, no un tokenizer exacto; al agotarla la barra lo dice y debes iniciar otra conversación con `/model`. Esa cota no prueba el contexto efectivo. `/model` consulta `/api/ps` para marcar qué modelos ya están en memoria; si esa consulta falla, la lista se muestra igual y todos figuran como no cargados. La medición de recursos sigue en los diagnósticos. Los valores incluidos dejan el servidor compartido: no se pide descargar modelos. `/liberate`, y el mismo criterio al arrancar, solo envían `keep_alive: 0` si `gpu.exclusive` está concedido y solo para modelos que esta sesión seleccionó. Lo demás se conserva. No se mata ningún proceso y no se promete dejar la GPU a cero. Si `nvidia-smi` no está en la ruta configurada ni en `C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe`, la medición se declara no disponible.

## Herramientas y tareas

El modelo con capacidad `tools` puede listar, buscar texto literal, leer UTF-8, crear sin sobrescribir y editar una coincidencia exacta usando el SHA-256 de una lectura previa. La raíz es el proyecto elegido; se rechazan escapes, enlaces, hardlinks múltiples, archivos binarios y áreas de control. El código/configuración de la propia instalación de pill permanece protegido. Tamaños, exclusiones, respaldos y pasos se configuran en `files` y `agents`.

La edición conserva la base en una reserva `.pill-transaction-*`. Entre el traslado del original y la publicación hay una ventana sin destino: una colisión conserva el ocupante externo y los respaldos y devuelve conflicto o resultado parcial. No hay transacción de varios archivos. Un efecto parcial bloquea más acciones con efectos en esa sesión; revisa diario y respaldos antes de reiniciar. No hay recuperación automática ni borrado de respaldos.

Los comandos se proponen mediante `commands.commands`: `id`, `program` absoluto, `args`, `cwd` relativo y `watched_files` con scripts/definiciones cuya modificación invalida el consentimiento. `/permissions` muestra una fila corta por propuesta: número, identificador y si está concedida, no concedida o rechazada. Las flechas marcan una fila; un número escrito también. Enter muestra el detalle completo. **Ctrl+Enter** concede la fila si no lo está y la retira si ya está concedida. Esc cierra sin cambiar concesiones. Una concesión no aprueba las demás. Una raíz extra se muestra y se rechaza: no amplía el alcance ni tiene concesión que retirar. La configuración por sí sola no concede nada. Las concesiones viven fuera del proyecto, en `%LOCALAPPDATA%/PILLCLI/trust.json`. Los comandos se ejecutan sin shell implícito, con salida limitada, timeout y cancelación de su árbol hijo propio. Un comando aprobado conserva los privilegios de tu usuario de Windows.

`mcp.servers` propone servidores stdio locales: `id`, `program` absoluto, `args`, `cwd` relativo y `env` como `CLAVE=valor`. Al menos un argumento debe ser un archivo dentro del proyecto; un archivo fuera de la raíz se rechaza. La concesión queda ligada al programa, los argumentos, el directorio y el contenido observado. Cambiar el script la invalida. Solo el implementador y el tester pueden llamar `mcp__servidor__herramienta`. Una anotación de solo lectura no autoriza la llamada. Un esquema con `$ref` externo se rechaza sin red y sin leer archivos arbitrarios. Si el servidor no responde o muere, la llamada no es un éxito. El chat usa el rol implementador, así que un servidor concedido puede aparecer en el chat. El explorador y el reviewer no reciben esas herramientas. Con la lista vacía, el trabajo nativo sigue disponible. No hay transporte HTTP remoto ni sampling.

`/task` activa para la próxima petición coordinador, explorador, implementador, tester y reviewer, con historiales separados. Necesita al menos una prueba registrada y concedida. `/chat` vuelve al chat con herramientas. Inferencias y herramientas se ejecutan en serie; las lecturas paralelas de R05 siguen pendientes. El cierre exige un comando real exitoso del tester, revalidación de archivos/definiciones probados y lectura de archivos cambiados con veredicto JSON APPROVE del reviewer. El ejecutor pide actividad real cuando un rol solo afirma haber actuado; una prueba fallida o CHANGES_REQUESTED vuelve al implementador, con reintentos acotados por `agents.max_steps`, también máximo de activaciones de roles de la tarea. Un modelo puede equivocarse o agotar límites: esos casos dejan la tarea pendiente, conservando sus efectos en el diario.

Los diarios `.pillcli/sessions/*.jsonl` registran rol, llamada, hashes, rutas, respaldos y resultados de comandos; no contienen el historial completo ni la salida completa. `/journal` muestra las últimas líneas del diario ya existente y no crea una sesión; si no hay ninguno, lo dice. Sirven para inspeccionar lo observado, sin reproducir operaciones al reiniciar. PgUp/PgDn desplazan la respuesta. Fuera del menú y de las listas, las flechas recuperan peticiones; dentro de una lista mueven la fila. La línea de una herramienta resume la ruta y, si cabe, las coincidencias, el cambio o el código de salida.

El contexto predeterminado sigue en 4096. La cota conservadora incluye esquemas y resultados de herramientas y puede agotarse tras pocas lecturas; el diagnóstico de herramientas usa un contexto explícito mayor, configurable también mediante `ollama.num_ctx`. Aumentarlo puede elevar el uso de memoria. No se selecciona otro modelo ni se recorta silenciosamente el historial para continuar.

Perfil verificado por el tester en consola: `huihui_ai/qwen3-abliterated:4b-v2`, `ollama.num_ctx=16384`, `ollama.num_predict=2048`, `ollama.think=true`. Los intentos con otros perfiles también produjeron fallos que el ejecutor detectó; el anuncio `tools` no garantiza que un modelo llame correctamente a herramientas. Este perfil de prueba no cambia los valores incluidos ni selecciona un modelo principal. Configúralo explícitamente si quieres reproducir la muestra:

```json
{"ollama":{"model":"huihui_ai/qwen3-abliterated:4b-v2","num_ctx":16384,"num_predict":2048,"think":true}}
```

Integraciones reales sobre una muestra temporal; se debe elegir explícitamente modelo, contexto y Python instalado:

```powershell
$env:PILL_LIVE_MODEL = 'NOMBRE_INSTALADO'
$env:PILL_LIVE_CONTEXT = '16384'
$env:PILL_LIVE_THINK = 'true'
$env:PILL_LIVE_PREDICT = '2048'
$env:PILL_TEST_PYTHON = (Get-Command python).Source
$env:PILL_LIVE_EVIDENCE = 'evidencias/herramientas-prueba.json'
cargo test --locked --offline --test tools_live model_repairs_project_with_tools -- --ignored --exact --nocapture
$env:PILL_LIVE_EVIDENCE = 'evidencias/agentes-prueba.json'
cargo test --locked --offline --test tools_live five_agents_repair_and_verify_project -- --ignored --exact --nocapture
```

## Desarrollo y comprobación

Requiere Rust 1.94 y herramientas MSVC. Las nueve dependencias directas incorporadas hasta E3/E4 y sus transitivas están fijadas en `Cargo.lock`; esta versión se compiló desde caché, sin descargas.

```powershell
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
cargo build --release --locked --offline
.\scripts\Install-Pill.ps1 -SkipBuild -SessionOnly
python diagnostics/test_console.py
```

La prueba Python usa solo biblioteca estándar, dos consolas reales ocultas y eventos Win32. Comprueba menú sin Enter, filtro, Esc, Tab, Unicode, redimensionado y restauración; guarda la regresión sobre E2 en `evidencias/2026-10-20-e2-regresion-consola.json`. El ancho en pantalla del texto no ASCII sigue siendo conservador. Retroceso, Supr e izquierda/derecha tratan como una sola unidad una familia ZWJ, las marcas, el selector de variación y una bandera; una consola puede seguir sin el glifo.

Las pruebas reales de Ollama son explícitas y cargan el modelo elegido para la prueba; no cambian el predeterminado del producto:

```powershell
$env:PILL_LIVE_MODEL = 'NOMBRE_INSTALADO'
$env:PILL_LIVE_EVIDENCE = 'evidencias/ollama-prueba.json'
cargo test --locked --offline --test ollama_live real_chat_and_errors -- --ignored --exact --nocapture
cargo test --locked --offline --test ollama_live real_cancel_and_retry -- --ignored --exact --nocapture
python diagnostics/test_ollama_console.py --model $env:PILL_LIVE_MODEL
```

Una prueba ignorada en la ejecución ordinaria de Cargo no cuenta como PASS de integración. Estas pruebas comprueban chat/streaming y cancelación; no acreditan herramientas ni calidad general de programación. API y campos de métricas: [chat](https://docs.ollama.com/api/chat), [modelos cargados](https://docs.ollama.com/api/ps).

Evidencia E2: [comparación de los tres candidatos](evidencias/2026-10-20-e2-comparacion.md), [consola real](evidencias/2026-10-20-e2-consola.json), [tester independiente](evidencias/2026-10-20-e2-tester.md) y [revisión](evidencias/2026-10-20-e2-review.md). El modelo predeterminado continúa sin elegir. La captura Win32 no acredita representación visual de todos los emojis; el diagnóstico final usa texto Unicode con acento y la edición sigue siendo por caracteres.

## Diagnóstico reproducible

El sondeo utiliza el mismo control de arranque y las dependencias Windows ya presentes:

```powershell
cargo run --locked --offline --manifest-path diagnostics/console-probe/Cargo.toml
cargo clippy --locked --offline --manifest-path diagnostics/console-probe/Cargo.toml --all-targets -- -D warnings
```

El código de salida 0 del sondeo significa que pudo ejecutarse y guardar su observación; la decisión de arranque se lee en `arranque_interactivo=permitido/rechazado`. No acredita interfaz ni funcionalidades de agente. Las capturas reales de terminal y consola propia están en `evidencias/2026-10-20-consola-*.txt`.

## Plan y reglas

[Plan aprobado](PLAN-IMPLEMENTACION.md) · [Principios](spec/00-PRINCIPIOS.md) · [Requisitos](spec/02-REQUISITOS.md) · [Pendientes](spec/03-PENDIENTES.md) · [Bitácora](memory.md).

Interacciones en español, inferencia local inicialmente con Ollama, modelos configurables y permisos explícitos para instalaciones y acciones destructivas. Las funcionalidades pendientes no se anuncian como implementadas.
