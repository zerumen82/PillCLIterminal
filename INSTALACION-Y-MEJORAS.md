# Instalacion y uso de pill

Guia del corte verificado de 2026-10-28. El estado ejecutado y los
veredictos se registran en [memory.md](memory.md); una evidencia antigua no
valida fuentes nuevas.

## Instalar o actualizar

Desde PowerShell, en una consola interactiva:

```powershell
cd D:\PROJECTS\PILLCLITER
.\scripts\Install-Pill.ps1 -SkipBuild -WhatIf
.\scripts\Install-Pill.ps1 -SkipBuild
Get-Command pill
pill --version
pill
```

El script valida version, SHA e identidad del binario, prepara su publicacion
sin sobrescribir ocupantes desconocidos y registra solo `bin` en el PATH del
usuario y de esa sesion. Conserva las demas entradas, sus espacios, expansiones,
orden y tipo del Registro. No escribe Machine PATH ni descarga paquetes/modelos.
`-SessionOnly` limita el registro al proceso actual. Sin `-SkipBuild` ejecuta
`cargo build --release --locked --offline`; requiere Rust/MSVC y cache local.

`bin/installation.json` acredita que entrada anadio esta instalacion. Si ya
existia, la registra como preexistente y no adquiere propiedad para retirarla.
Una version anterior sin manifiesto o una sustitucion desconocida exige revisar
el SHA real y aportar `-ExpectedPreviousHash SHA256_VERIFICADO`. No copies un
SHA arbitrario para saltar un conflicto. Las reservas de actualizacion se
conservan para revisar; no se borran carpetas recursivamente.

Otras aplicaciones ya abiertas pueden conservar un PATH antiguo. Abre una
consola que herede el entorno actualizado. `pill --project RUTA` selecciona
otro proyecto; sin ese argumento usa la carpeta actual.

## Retirar el registro

Primero simula:

```powershell
.\scripts\Uninstall-Pill.ps1 -WhatIf
```

Sin `-WhatIf`, el desinstalador retira exclusivamente la entrada propia
acreditada. Conserva entradas ajenas o preexistentes, proyectos, sesiones y
modelos. Si detecta duplicados, cambios o una instalacion incompleta, informa
el conflicto y se detiene. La opcion separada `-RemoveBinary` solicita ademas
borrar solo el ejecutable con el hash e identidad registrados. No elimina
`bin` ni el proyecto. La desinstalacion real del usuario no se ejecuta como
parte de las pruebas de este corte.

`-WhatIf` no compila, ejecuta binarios, crea reservas ni cambia el PATH.
Las comprobaciones del instalador y desinstalador usan muestras temporales y
PATH de proceso. El registro User real se ejecuto tras PASS/APPROVE el 2026-10-28,
conforme a la autorizacion expresa recibida. El comando pill resolvio la
version instalada desde un proceso nuevo; Machine PATH se conservo.

## Consola y fiabilidad

- `/` despliega comandos; flechas seleccionan, Tab completa y Esc cierra.
  Ctrl+Enter envia peticiones; Enter agrega linea. Ctrl+C restaura y sale.
- `/ask` permite solo lecturas; `/edit` habilita las herramientas de edicion
  autorizadas; `/chat` es su alias. `/task` coordina cinco roles con pruebas.
- `/permissions` muestra acciones legibles. Enter abre detalle y huella;
  Ctrl+Enter concede o retira. Mover/borrar requiere concesion concreta de un uso.
- `/changes` muestra cambios verificados de la sesion y sus respaldos. `/undo`
  exige conservar el SHA posterior y confirmar el respaldo concreto. No hay
  transaccion de varios archivos ni deshacer universal.
- `/recover` contrasta diario y disco sin repetir efectos. Una revision humana
  durable reconoce la evidencia mostrada; no convierte `unknown` en exito.
- `/archive` propone mover evidencia revisada, nunca destruirla. Las cuotas
  desconocidas o exploraciones incompletas se muestran como tales. No hay
  limpieza automatica de respaldos.
- `/status` distingue seleccionado, cargado y propiedad; metadatos invalidos
  dejan estado desconocido. `/profile` ofrece `ligero` y `codigo`, sin elegir
  modelo ni guardar un nuevo predeterminado. Un perfil incompatible se rechaza.
- `/copy` o Ctrl+Y copia la ultima respuesta completa; Ctrl+R abre razonamiento.
  `/journal` muestra el diario. `/liberate` exige Ctrl+Enter y exclusividad,
  y conserva modelos/procesos ajenos.

La cabecera y capsula son compactas. `ui.compact_header=false` conserva el dibujo
completo; `ui.ascii_logo=true` usa ASCII. El historial esta acotado por bytes y
entradas; sus fallos de guardado permanecen visibles. Se conservan turnos completos
y un respaldo valido recuperable. El ancho de glifos es conservador, segun fuente.

Solo consultas de metadatos admiten reintentos automaticos acotados. Una generacion,
comando o efecto MCP desconocido nunca se repite automaticamente. El cliente
separa carga/primer fragmento, inactividad y limite total por configuracion.
Los MCP y comandos concedidos tienen los permisos reales del usuario; no son
procesos aislados. Los trabajadores de lectura se reutilizan, el render escribe
filas cambiadas y el catalogo MCP conserva validacion de huellas y permisos.

## Verificacion

```powershell
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
cargo build --release --locked --offline
.\diagnostics\test_installation.ps1 -Evidence evidencias/instalacion-nueva.json
```

Las pruebas con modelos son opt-in y necesitan modelo, Python y evidencia
explicitos. [README.md](README.md) documenta esos parametros. No cuentan como
PASS por estar ignoradas. El modelo principal continua sin elegir y E6/v1
conserva las decisiones pendientes del usuario.
