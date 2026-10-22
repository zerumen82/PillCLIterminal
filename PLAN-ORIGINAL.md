  quierevisa e# PLAN ORIGINAL · Agente de programación local para terminal

> **Visión:** una cápsula verde en la consola que convierte instrucciones en cambios verificables en el proyecto, sin ceder el control del equipo.
>
> **Estado:** referencia histórica, actualizada el 2026-10-20. Aprobados: Rust, terminal con cápsula propia, multiagentes, cliente MCP `stdio`, lectura/escritura sin confirmación por archivo y ejecución de pruebas/compilación dentro del proyecto. Instalaciones y acciones destructivas requieren confirmación. Búsqueda web después del ciclo local. Los tres modelos Huihui ya están descargados; no hay implementación del CLI. El [plan técnico](PLAN-IMPLEMENTACION.md) y `spec/00` a `spec/04` concretan las propuestas pendientes de aprobación.

## Decisiones actuales · 2026-10-20

- Interfaz de terminal, con identidad propia basada en una cápsula e inspiración en la experiencia de otros agentes CLI. Diseño exacto pendiente.
- Comandos: al escribir `/` en el campo de entrada se desplegará automáticamente el listado de comandos disponibles, incluido `/liberate`, sin pulsar Enter para abrirlo. Mostrar el listado no ejecuta comandos.
- Lectura y escritura: autorización permanente del usuario, sin pedir confirmación por cada archivo. Raíces concretas propuestas en el nuevo plan. Pruebas y compilación dentro del proyecto aprobadas; instalaciones y acciones destructivas requieren confirmación. Las referencias posteriores a confirmaciones describen la política inicial propuesta y no exigen repetir permisos ya concedidos.
- Recursos: Rust aprobado para priorizar bajo consumo. Hardware comprobado: aproximadamente 32 GiB de RAM y RTX 3060 Ti de 8 GiB. No hay mediciones de consumo del futuro CLI ni comparativa que permita prometer un ahorro numérico.
- Red: búsqueda web aplazada por aprobación del usuario hasta validar el ciclo local. MCP HTTP remoto sigue fuera de v1.
- Servidor: Ollama responde localmente. Valorar `llama.cpp` como comparación con el mismo modelo y parámetros; no se afirma que sea más rápido sin medirlo.
- Modelos: el usuario requiere variantes publicadas como sin censura (`uncensored`/`abliterated`) y autorizó sustituir los tres candidatos normales. Descargados y verificados por API: `huihui_ai/qwen3.5-abliterated:4B`, `huihui_ai/qwen3-abliterated:4b-v2` y `huihui_ai/qwen3.5-abliterated:2B`, todos Q4_K_M y con capacidad `tools` anunciada localmente. Los tres candidatos normales están ausentes y los modelos preexistentes se conservan. Son modificaciones comunitarias; la etiqueta no acredita ausencia absoluta de rechazos, calidad de programación conservada ni fiabilidad de herramientas. Evaluación mediante inferencia y elección del modelo principal pendientes. Evidencia: `evidencias/2026-10-20-verificacion-abliterated.json`.
- GPU: el usuario concreta dos disparadores de liberación de VRAM: al abrir el CLI y mediante el comando exacto `/liberate`. No habrá limpieza periódica. Se descargarán únicamente recursos que se puedan liberar sin interrumpir tareas activas ni afectar procesos ajenos; si hay trabajo activo, se esperará a un punto seguro. En servidores compartidos hay que comprobar que el recurso no está en uso por otro cliente; `/api/ps` por sí solo no demuestra exclusividad. El detalle de coordinación queda pendiente de especificar. Se informará de la VRAM observada antes/después y de los recursos que se conserven por estar en uso o no poder verificar su propiedad; no se promete dejar la GPU a cero.
- Idioma: interacciones del asistente y bitácora `memory.md` en español; conservar identificadores técnicos, comandos y nombres propios.
- Instantánea anterior: `/api/ps` devolvió cero modelos cargados en Ollama aunque la GPU usaba unos 4,4 GiB. El Python de cómputo se identificó después como ComfyUI de AUTOAUTO; el diagnóstico posterior está en [evidencias/2026-10-20-gpu-final.json](evidencias/2026-10-20-gpu-final.json). Las cifras son capturas fechadas y no describen permanentemente el estado de la GPU.

Las decisiones de este apartado prevalecen sobre las propuestas iniciales del documento. Ya se han redactado `spec/00` a `spec/04` y [PLAN-IMPLEMENTACION.md](PLAN-IMPLEMENTACION.md) como borradores concretos para aprobación antes de implementar e instalar paquetes Rust.

Revisión técnica del 2026-10-20: [Ollama, agentes y operaciones reales de archivos](REVISION-OLLAMA-ARCHIVOS.md). Contiene evidencia local y contratos propuestos para creación, edición, corrección, movimiento y eliminación; sus propuestas quedan pendientes de aprobación y no acreditan funcionalidad implementada.

---

## 🟢 La idea en una frase

Crear un CLI para Windows que use un LLM local intercambiable, coordine agentes especializados, se conecte a herramientas MCP, entienda el proyecto abierto, busque información en Internet cuando se permita y pueda proponer, crear o modificar archivos, ejecutar comprobaciones y explicar qué verificó.

**No será solo un chat ni un wrapper de un gestor de modelos.** El valor del producto estará en el ciclo completo:

```text
coordinar → inspeccionar → delegar → proponer → pedir permiso → actuar → comprobar → informar
```

## 🎯 Qué significa “útil de verdad”

Una tarea se considera terminada solamente cuando el agente:

1. Entiende el objetivo y trabaja en la carpeta correcta.
2. Encuentra los archivos pertinentes en vez de inventar su ubicación.
3. Explica brevemente el cambio que planea hacer.
4. Presenta cambios revisables y solicita autorización antes de escribir.
5. Ejecuta pruebas o comprobaciones apropiadas cuando el usuario lo autoriza.
6. Comunica con honestidad los resultados, errores y verificaciones no realizadas.

La calidad no se mide por lo convincente que suena la respuesta. Se mide por cambios correctos, pruebas reales y control explícito de las acciones.

## 🖥️ Experiencia de consola

La cápsula verde sería la marca visual propia del CLI; no se reutilizarán el logo ni otros recursos distintivos de Claude Code.

```text
       ╭──────────────────────────────╮
       │  ●  PILLCLI                  │
       │  agente local · D:\mi-proyecto │
       ╰──────────────────────────────╯

  Modelo   Ollama / modelo elegido
  Red      desactivada o proveedor configurado
  Sesión   lista

  Tú > Añade validación al formulario de registro.

  Agente
    Revisaré el formulario y las pruebas relacionadas.
    Después propondré el cambio y te enseñaré el diff.

  ✓ Búsqueda del proyecto
  ✓ Lectura de archivos relevantes

  Cambio propuesto
    M src\...
    + ...
    - ...

  ¿Aplicar el cambio?  [Sí] [No] [Ver diff completo]
```

### Comportamiento de la interfaz

- Desplegar automáticamente el listado de comandos disponibles al escribir `/` en el campo de entrada; incluir `/liberate` y abrir el menú sin ejecutar ninguna acción.
- Mostrar el modelo, el proveedor y la ruta de trabajo activos.
- Transmitir la respuesta gradualmente y permitir cancelar una operación.
- Enseñar cada herramienta que se ejecuta, su objetivo y su resultado.
- Presentar diffs legibles antes de escribir archivos.
- Mantener conversación e historial de tareas recuperables.
- Soportar PowerShell en Windows; no suponer que Bash está instalado.
- Empezar con una TUI modesta; añadir una interfaz rica solo si mejora el flujo.

## 🧱 Arquitectura propuesta

```text
┌──────────────────────────────────────────┐
│ Interfaz CLI                             │
│ prompt · progreso · diff · confirmaciones│
└─────────────────────┬────────────────────┘
                      │
┌─────────────────────▼────────────────────┐
│ Coordinador de agentes                    │
│ planifica · delega · integra · verifica    │
└─────────┬─────────────────────┬───────────┘
          │                     │
┌─────────▼──────────┐  ┌───────▼──────────────┐
│ Agentes especializados│ │ Bucle de herramientas│
│ explorar · implementar│ │ validar · resultado  │
│ probar · revisar      │ │ · siguiente paso     │
└──────────┬─────────┘  └───────┬──────────────┘
           │                    │
┌──────────▼────────┐  ┌────────▼────────────────┐
│ Adaptador LLM     │  │ Registro de herramientas│
│ Ollama / LM Studio│  │ nativas + cliente MCP   │
│ / llama.cpp server│  │ archivos · shell · web  │
└───────────────────┘  └────────┬────────────────┘
                                │
                      ┌─────────▼──────────┐
                      │ Permisos y límites │
                      │ por agente/acción  │
                      └────────────────────┘
```

### Módulos principales

| Módulo | Responsabilidad |
|---|---|
| Interfaz terminal | Leer instrucciones, mostrar el estado, diffs, permisos y resultados. |
| Coordinador multiagente | Descomponer la tarea, asignar subtareas, recoger resultados y evitar ediciones concurrentes en conflicto. |
| Agentes especializados | Ejecutar roles acotados (explorar, implementar, probar, revisar) con contexto, herramientas y límites explícitos. |
| Orquestador | Gestionar mensajes, límites de pasos, concurrencia, cancelación, errores y el ciclo de herramientas de cada agente. |
| Proveedor de modelo | Conectar con un servidor local configurable; convertir respuestas y errores a un formato interno común. |
| Contexto | Determinar carpeta activa, instrucciones del proyecto, archivos relevantes y límites de lectura. |
| Herramientas y MCP | Exponer capacidades nativas y conectar servidores MCP configurados, validando entradas y salidas. |
| Permisos | Autorizar o rechazar acciones según su riesgo y el alcance aprobado; los agentes hijos y servidores MCP no heredan privilegios implícitos. |
| Historial | Guardar sesiones y evidencia de acciones localmente, sin guardar secretos por accidente. |

### Multiagentes — requisito de primera versión

La v1 incluirá varios agentes bajo un **coordinador central**, no agentes autónomos sin supervisión:

- **Coordinador:** entiende el pedido, propone un plan de subtareas, decide dependencias y conserva la responsabilidad de la respuesta final.
- **Explorador:** busca y lee código; devuelve rutas, hallazgos y evidencia. Sus herramientas son de solo lectura.
- **Implementador:** propone cambios para subtareas asignadas. Las escrituras se integran de forma controlada; no se permiten cambios concurrentes no coordinados sobre los mismos archivos.
- **Tester:** recomienda y ejecuta comprobaciones aprobadas; informa resultados observados, no inferidos.
- **Revisor:** inspecciona diffs y criterios de aceptación; no aplica arreglos por su cuenta.

El coordinador puede paralelizar subtareas independientes, empezando por exploración y lectura. Cambios de archivos y comandos con efectos secundarios se serializan o se entregan como propuestas para que el coordinador las integre y solicite aprobación. Cada agente tiene límites configurables de pasos, tiempo, contexto y concurrencia.

La concurrencia real dependerá de la memoria disponible y del runtime local. El sistema debe poder reducirla o ejecutar subtareas en serie; no prometerá que varios agentes implican más velocidad. En v1 los agentes pueden compartir el proveedor/modelo configurado, sin exigir descargas de varios modelos.

## 🔌 Gestores y motores de LLM

El CLI no debe depender de la marca del gestor. Todos los proveedores se evaluarían con el mismo conjunto de tareas, modelo compatible y, cuando sea posible, cuantización y parámetros equivalentes.

### Ranking práctico para este proyecto

Este orden prioriza integración con un agente CLI en **Windows**, disponibilidad de API local, herramientas y facilidad de evaluación. **No es un ranking de velocidad ni de calidad de modelos.**

| Puesto | Opción | Papel | Evaluación |
|---:|---|---|---|
| **1** | **Ollama** | Gestor y runtime local | Punto de partida: ya está instalado y expone API local y tool calling. Debe ser la línea base, no el ganador supuesto. |
| **2** | **LM Studio** | Gestor de escritorio y servidor local | Candidato para explorar modelos cómodamente y probar una API local con herramientas. |
| **3** | **llama.cpp server** | Motor y API local | Control directo de GGUF, CPU/GPU y cuantización; referencia para comparar rendimiento y compatibilidad. |
| **4** | **Jan** | Aplicación de escritorio y API local | Alternativa abierta para administrar modelos y experimentar; validar llamadas de herramientas con los modelos elegidos. |
| **5** | **LocalAI** | Servidor de modelos con varios backends | Interesante si se necesita una API unificada sobre varios motores; añadirlo solo si aparece esa necesidad. |
| **6** | **text-generation-webui** | Interfaz de experimentación y servidor | Flexible y con múltiples backends; más apropiado como laboratorio que como requisito del CLI inicial. |
| **7** | **KoboldCpp** | Runtime portable GGUF | Fácil de probar en Windows; no asumir que su agente integrado sustituye al harness propio. |
| **8** | **GPT4All** | Aplicación de chat y API local | Sencillo para chat local; validar específicamente API y tool calling antes de considerarlo proveedor del agente. |
| **9** | **llamafile** | Ejecutable portable de inferencia | Útil para distribución autocontenida, pero su documentación señala un límite de tamaño de ejecutable para Windows. |

**Herramientas de enrutamiento, no primera opción de runtime:**

- **llama-swap:** enruta entre servidores y facilita cambiar de modelo; útil si se gestionan varios backends.
- **LiteLLM:** unifica proveedores y puede ofrecer fallback/routing; gana sentido al incorporar endpoints cloud o una infraestructura con varios proveedores.
- **vLLM:** servidor de alto rendimiento y concurrencia, orientado principalmente a Linux/servidores; Windows nativo no es su ruta oficial y añadir WSL para un CLI individual no parece una primera dependencia.

**Decisión pendiente:** mantener Ollama como única integración inicial o implementar desde el comienzo un adaptador compatible con más de un servidor. Recomendación para discutir: abstraer el contrato interno, validar Ollama primero y sumar otro proveedor solo mediante una prueba de aceptación real.

## 🧰 Herramientas del agente

### Archivos y navegación del proyecto

- **Listar:** mostrar estructura sin recorrer dependencias o salidas generadas por defecto.
- **Leer:** extraer archivos relevantes con límites de tamaño y rutas visibles.
- **Buscar:** encontrar nombres, símbolos y texto; se puede usar `ripgrep` (`rg`) en Windows o una implementación interna.
- **Crear y editar:** generar cambios propuestos y mostrar diff antes de aplicarlos.
- **Protección de rutas:** rechazar rutas fuera del proyecto aprobado, traversal y escrituras a archivos sensibles configurados.

### Terminal y verificación

- Ejecutar comandos de PowerShell con directorio de trabajo y argumentos visibles.
- Detectar scripts disponibles en el proyecto para pruebas, lint y build; no inventar comandos si no hay configuración.
- Solicitar confirmación antes de ejecutar comandos, especialmente los que cambian el equipo o descargan dependencias.
- Capturar código de salida y salida relevante; no declarar éxito si el comando falló.
- Empezar sin instalaciones automáticas de paquetes.

### Git

- Consultar estado, diff y archivos modificados para ayudar a revisar el cambio.
- Nunca sobrescribir cambios preexistentes del usuario.
- No hacer commits, reset, checkout destructivo ni force-push sin aprobación explícita.

### Búsqueda en Internet

Buscar y leer páginas son capacidades distintas. El CLI puede buscar resultados mediante un proveedor y leer las páginas elegidas para preparar una respuesta con enlaces y fuentes.

| Estrategia | Ventajas | Costes y límites |
|---|---|---|
| **Brave Search API** | Servicio pensado para entregar resultados de búsqueda a aplicaciones. | Necesita Internet y credencial; revisar cuota, precio y términos al seleccionar la integración. |
| **Tavily** | API de búsqueda/extracción orientada a agentes y flujos de investigación. | Necesita Internet y API key; revisar condiciones y límites de uso. |
| **SearXNG propio** | Se puede alojar una instancia propia como metabuscador. | Hay que instalarla y mantenerla; las consultas siguen dependiendo de motores externos. |
| **Solo lectura HTTP** | No necesita una API de búsqueda para abrir una URL conocida. | No descubre resultados y algunos sitios bloquean o requieren JavaScript. |

**Propuesta inicial:** herramienta de búsqueda web opcional, desactivable y con proveedor configurable. La clave se guarda fuera del código. El modelo puede ser local mientras la búsqueda usa Internet; por tanto, “LLM local” no equivale a “operación completamente offline”.

Contenido obtenido de Internet debe tratarse como **dato no confiable**, nunca como una orden capaz de cambiar permisos o ejecutar comandos.

### MCP — requisito de primera versión

MCP ofrece un estándar para conectar herramientas, recursos y flujos externos ([introducción oficial](https://modelcontextprotocol.io/introduction)). La v1 incluirá un **cliente MCP con transporte `stdio` para servidores locales**, además de las herramientas nativas del CLI.

- Cargar servidores desde configuración local explícita; no instalar ni iniciar servidores desconocidos automáticamente.
- Mostrar nombre del servidor y herramientas expuestas; validar esquemas y argumentos antes de invocar.
- Aplicar las mismas reglas de permisos del CLI: las herramientas MCP no reciben permisos más amplios por ser externas.
- Solicitar aprobación para operaciones que escriban, ejecuten comandos, accedan a red o tengan efectos secundarios.
- Permitir deshabilitar un servidor y mostrar errores de inicio, protocolo o invocación sin ocultarlos.
- En v1, no se implementan transportes MCP remotos por HTTP; quedan para una ampliación posterior que defina autenticación, confianza y seguridad de red.

## 🔐 Seguridad y control del usuario

| Riesgo/acción | Control propuesto |
|---|---|
| Lectura fuera de alcance | Limitar por defecto el acceso a la carpeta de trabajo y exclusiones configuradas. |
| Exposición de secretos | Excluir archivos de credenciales y ocultar patrones sensibles en contexto y logs. |
| Modificación inesperada | Revisar el diff y pedir permiso antes de escribir. |
| Comandos peligrosos | Mostrar comando y directorio, pedir aprobación; bloquear operaciones claramente destructivas por defecto. |
| Instrucciones maliciosas desde la web o archivos | Separar datos observados de instrucciones autorizadas; exigir confirmación para actuar. |
| Cambios del usuario en el worktree | Detectar estado previo y preservar cualquier cambio que no haya creado el agente. |
| Fallo o respuesta inválida del modelo | Validar schema, señalar el error y parar; nunca convertir una llamada inválida en una operación válida silenciosamente. |

La ejecución en sandbox sería una opción adicional, no una garantía automática del CLI. Docker Desktop/WSL y Windows Sandbox añaden instalación, requisitos y límites propios; escoger uno requiere decidir qué aislamiento se busca y dónde deben persistir los cambios.

## 📦 Requisitos de instalación propuestos

### Desarrollo del producto

| Componente | Estado | Para qué |
|---|---|---|
| Windows y PowerShell | Disponible en el entorno objetivo | Interfaz y ejecución inicial de comandos. |
| Node.js 24 LTS + npm | **Propuesto, pendiente de aprobación** | Runtime y dependencias del CLI en TypeScript. Node clasifica v24 como LTS en su calendario consultado. |
| Git for Windows | Recomendado | Historial y revisión de archivos; no habilita acciones destructivas sin permiso. |
| Editor | Recomendado | Desarrollo y revisión manual del producto. |

Ink/React puede usarse para la interfaz de terminal si se aprueba una experiencia más rica; no es obligatorio para la primera versión.

### Uso del agente local

| Componente | Necesidad |
|---|---|
| **Un servidor local de LLM** | Necesario para inferencia local: inicialmente Ollama, o una alternativa aprobada. |
| **Un modelo local** | Necesario; selección pendiente de evaluar herramientas, contexto, memoria y rendimiento en el equipo comprobado: 32 GiB de RAM y RTX 3060 Ti de 8 GiB. |
| **Internet** | Solo para búsqueda web, descarga de modelos/dependencias o proveedores remotos. |
| **API key de búsqueda** | Solo si se elige un servicio como Brave o Tavily. |
| **`rg`** | Opcional si se adopta ripgrep para búsquedas más rápidas. |
| **Docker/WSL o Windows Sandbox** | Opcional si se aprueba aislar la ejecución. |
| **Servidor MCP local** | Opcional por configuración; solo si se quiere conectar una herramienta MCP concreta. La capacidad cliente MCP del CLI sí forma parte de la v1. |
| **SDK/paquete cliente MCP** | Dependencia de desarrollo probable para la integración `stdio`; seleccionar e instalar solo tras aprobar stack y paquete. |
| **Agentes y modelo local** | No requieren gestores adicionales; los agentes comparten el proveedor configurado. La concurrencia puede reducirse según la memoria disponible. |

No instalar todos los gestores. Mantener uno de base y sumar otros únicamente para comparaciones aprobadas. El CLI debe reportar claramente cuando falta un servicio, modelo o credencial.

## 🧪 Cómo elegir modelo y gestor

Separar tres preguntas:

1. **Calidad del modelo:** ¿resuelve la tarea y genera cambios correctos?
2. **Calidad de herramientas:** ¿solicita herramientas válidas, con argumentos bien formados y en el orden correcto?
3. **Rendimiento del runtime:** ¿qué latencia, tokens/s y consumo de memoria ofrece en este equipo?

Comparar con un conjunto fijo de tareas locales: comprender un repo, arreglar un bug con prueba existente, cambiar varios archivos y añadir una prueba. Registrar si compila, si pasan pruebas, si el agente se equivocó de herramienta, tiempo, memoria y configuración exacta. Aider Polyglot y SWE-bench pueden aportar contexto externo, pero sus resultados no sustituyen la comparación en el hardware y harness propios.

## 🗺️ Roadmap sugerido

### Fase 0 · Acordar decisiones

- Aprobar stack de desarrollo, política de permisos, formato de configuración MCP y límites de agentes.
- Confirmar características del equipo antes de recomendar modelos o cuantizaciones.
- Decidir si búsqueda web será servicio externo con key, SearXNG propio o fase posterior.

### Fase 1 · CLI coordinado que entiende el repositorio

- Interfaz con cápsula/logotipo provisional, conversación y cancelación.
- Conexión configurada al LLM local elegido.
- Estado de proveedor/modelo y errores de conexión visibles.
- Listado, lectura y búsqueda de archivos con exclusiones seguras.
- Coordinador y agentes exploradores de solo lectura; delegación paralela limitada y progreso visible.
- Cliente MCP `stdio`: cargar servidores locales configurados, enumerar herramientas e invocarlas con permisos.

### Fase 2 · Cambios multiagente revisables

- Herramienta para proponer y aplicar modificaciones.
- Diff antes de escribir; aprobación explícita.
- Detección de cambios previos para proteger el trabajo del usuario.
- Agentes implementadores producen propuestas; el coordinador integra cambios y evita escrituras concurrentes incompatibles.

### Fase 3 · Verificación

- Herramienta de terminal PowerShell con permisos y salida capturada.
- Descubrimiento de pruebas y comprobaciones configuradas por el proyecto.
- Informe separado de pruebas pasadas, fallidas y no ejecutadas.
- Agentes tester/revisor con informes vinculados a comandos, salidas y diffs observables.

### Fase 4 · Internet

- Búsqueda y lectura de páginas con citas/enlaces.
- Configuración de proveedor, key local y opción de desactivar red.
- Protección ante instrucciones maliciosas en contenido externo.

### Fase 5 · Capacidades avanzadas

- Transporte MCP remoto/HTTP, tras diseñar autenticación, confianza y permisos.
- Aislamiento de ejecución optativo.
- Sesiones persistentes, compactación de contexto y selección de varios modelos.
- Worktrees aislados para cambios paralelos, solo si pruebas demuestran que compensan su complejidad.

## ✅ Criterios de aceptación del primer agente funcional

- Al escribir `/` en el campo de entrada, aparece el listado de comandos disponibles sin pulsar Enter; incluye `/liberate` y no ejecuta comandos por el mero hecho de mostrarlo.
- Se conecta a un modelo local configurable y muestra fallos reales de conexión.
- Inspecciona un proyecto sin leer por defecto carpetas ignoradas o sensibles.
- Encuentra un archivo pertinente mediante búsqueda local.
- Propone un cambio visible antes de aplicarlo.
- No escribe hasta recibir autorización explícita.
- Ejecuta una comprobación con aprobación y comunica su código de salida real.
- Conserva los cambios preexistentes del usuario.
- Puede completar una tarea sin Internet cuando la búsqueda web está desactivada.
- Al activar búsqueda web, devuelve fuentes y no ejecuta instrucciones encontradas en ellas.
- El coordinador puede delegar subtareas a agentes especializados, informar su estado y reunir sus resultados.
- Las subtareas de lectura independientes pueden correr en paralelo dentro del límite configurado; las modificaciones no colisionan ni se aplican sin permiso.
- El CLI limita pasos/concurrencia de agentes y puede degradar a ejecución serial cuando el hardware lo requiera.
- El cliente MCP `stdio` carga solo servidores configurados explícitamente, valida llamadas y aplica confirmaciones según riesgo.
- Una herramienta MCP no puede saltarse la política de permisos del CLI; errores del servidor se muestran y no se convierten en éxito.
- El agente puede completar una tarea básica con MCP desactivado y sin servidores configurados.
- Tiene pruebas, lint/build elegidos para el stack aprobado y evidencia registrada en `memory.md`.
- Al iniciar y al recibir `/liberate`, intenta liberar recursos de inferencia que pueda descargar sin afectar trabajo activo o clientes ajenos, y muestra memoria observada antes/después; no ejecuta limpieza periódica ni promete VRAM cero. La prueba debe comprobar también que un recurso compartido o de propiedad incierta se conserva.

## ⚠️ Riesgos y decisiones pendientes

- RAM, GPU y VRAM están comprobadas; falta comparar modelos y configuraciones con pruebas reales en este equipo antes de elegir uno.
- Los protocolos compatibles no garantizan tool calling idéntico en todos los runtimes/modelos.
- La concurrencia multiagente aumenta consumo de contexto y puede competir por RAM/VRAM; límites y modo serial son necesarios.
- Cada servidor MCP añade herramientas y código externo al perímetro de confianza; configuración, revisión y permisos son obligatorios.
- La búsqueda web depende de Internet; algunos proveedores requieren API key y pueden tener cuota/coste.
- Permitir shell sin aislamiento da al agente capacidad de afectar el equipo; permisos y diseño de ejecución importan más que el logo o el gestor.
- El código de Claude Code indicado por el usuario se estudiará solo como referencia arquitectónica. No copiar implementación, prompts ni recursos propietarios; verificar licencias antes de reutilizar cualquier componente.
- El repositorio de destino sigue sin código de aplicación, pruebas, paquetes ni tooling. Esta propuesta no establece todavía comandos de build, test o lint.

## 🔎 Fuentes oficiales consultadas

- [Node.js: calendario de releases](https://nodejs.org/en/about/previous-releases)
- [Ink: interfaz React para terminal](https://github.com/vadimdemedes/ink)
- [Ollama: API](https://docs.ollama.com/api/introduction) · [tool calling](https://docs.ollama.com/capabilities/tool-calling)
- [LM Studio: documentación de desarrollo y APIs](https://lmstudio.ai/docs/developer) · [tool use](https://lmstudio.ai/docs/developer/openai-compat/tools)
- [`llama.cpp` server](https://github.com/ggml-org/llama.cpp/tree/master/tools/server)
- [Brave Search API](https://brave.com/search/api/) · [Tavily API](https://docs.tavily.com/documentation/api-reference/introduction) · [SearXNG](https://docs.searxng.org/)
- [MCP: introducción](https://modelcontextprotocol.io/introduction)
- [MCP: documentación oficial](https://modelcontextprotocol.io/docs)
- [Docker Desktop en Windows](https://docs.docker.com/desktop/setup/install/windows-install/)
- [Windows Sandbox](https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/)
- [Aider: leaderboard](https://aider.chat/docs/leaderboards/) · [SWE-bench](https://www.swebench.com/)

Las opciones y enlaces describen las fuentes consultadas para elaborar esta propuesta; disponibilidad, precios, condiciones, versiones y compatibilidad deben reconfirmarse antes de implementar o instalar.
