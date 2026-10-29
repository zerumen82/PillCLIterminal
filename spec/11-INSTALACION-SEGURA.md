# Instalación y retirada seguras de pill

Diseño aprobado en la petición integral del 2026-10-27 y en
`10-USABILIDAD-FIABILIDAD-INSTALACION.md` (U13/U14). El usuario autorizó el
registro persistente final después de las pruebas y revisión. Durante el
desarrollo solo se probará PATH de proceso en árboles temporales.

1. `Install-Pill.ps1` admite `-SkipBuild`, `-SessionOnly`, `-WhatIf` y un hash
   anterior explícito para adoptar una instalación antigua sin manifiesto.
   Se derivan raíz/binario desde el directorio del script, y versión desde el
   manifiesto Cargo; ningún argumento permite escribir en otra instalación.
2. Un módulo común concentra comparación, publicación y propiedad. Los límites
   de espera y tamaño se validan desde `scripts/installation.defaults.json`.
   No se instalan módulos, paquetes o componentes del sistema.
3. Antes de publicar se comprueban source/hash/versión. Temporales, archivos y
   padres se abren con handles que rechazan enlaces y escrituras concurrentes.
   El archivo anterior se renombra a una reserva propia; el nuevo se publica
   sin reemplazo. Si aparece otro destino, no se sobreescribe ni restaura encima.
   No se promete atomicidad entre binario, manifiesto y Registro.
4. El manifiesto `bin/installation.json` vincula raíz, binario, versión y hash.
   Diferencia entradas añadidas de entradas preexistentes. La propiedad de
   proceso incluye PID y fecha de creación. Un manifiesto ajeno/corrupto se
   rechaza y nunca concede borrar rutas tomadas de su texto.
5. El PATH se conserva como texto: orden, entradas vacías, citas y expansiones
   quedan intactos. La comparación semántica de Windows evita duplicados por
   mayúsculas, barras finales o variables equivalentes. No se deduce propiedad
   de una entrada que ya existía, aunque sea equivalente a `bin`.
6. El alcance persistente solo abre `HKCU\Environment\Path`; conserva tipo
   String/ExpandString y lee sin expandir variables. Machine PATH no se escribe.
   Se verifica el valor justo antes y después; una diferencia produce conflicto.
   Un rollback solo actúa si el valor sigue siendo exactamente el propio escrito.
7. El registro persistente actualiza también esta sesión y notifica el cambio
   al sistema; las aplicaciones ya abiertas pueden necesitar otra consola.
   `-SessionOnly` trabaja exclusivamente con el entorno del proceso.
8. `Uninstall-Pill.ps1` retira únicamente la entrada añadida y acreditada por el
   manifiesto. Conserva entradas preexistentes, otras copias y todas las demás
   rutas. Si hay varias coincidencias o cambió la entrada se detiene: no adivina.
   Su acción predeterminada no borra binario, proyecto, sesiones ni modelos.
9. `-RemoveBinary` añade una acción explícita e independiente sujeta a
   ShouldProcess; exige ruta, hash e identidad propios antes de borrar por
   handle. No existe borrado recursivo. `-WhatIf` no crea directorios, archivos,
   concesiones o valores, ni compila ni ejecuta binarios.
10. Las pruebas de PowerShell usan biblioteca estándar y un binario real en
    muestras temporales. Cubren idempotencia, expansiones, entradas vacías,
    preexistentes, conflictos, manifiestos ajenos, simulación, retirada y
    eliminación condicionada. La prueba persistente real queda para el
    coordinador al final, con evidencia y veredictos independientes.

El mutex coordina instancias de este instalador, no todos los programas que
pueden modificar el entorno. La comprobación antes/después reduce carreras;
no constituye una transacción global con aplicaciones externas. Fallos
parciales conservan las reservas y describen el paso que quedó pendiente.


Correccion verificada en la instalacion real de 2026-10-28: Cargo puede publicar
`target/release/pill.exe` como hardlink de `target/release/deps/pill.exe`.
Solo el lector de ese artefacto compilado admite mas de un enlace, con acceso
GENERIC_READ|SYNCHRONIZE, sin WRITE/DELETE y compartiendo unicamente READ.
El bloqueo impide escribir los bytes por sus alias durante hash y copia.
Windows puede permitir retirar otro nombre de hardlink; no se promete bloquear
cambios de namespace ajenos. La copia usa el handle propio fijado y sincronizado. Se rechazan directorios/reparse, se fijan padres y se verifica la copia.
El binario destino, temporales y manifiestos mantienen requisito de un enlace.
No se cambia la politica de ediciones/borrado ni se elimina ningun alias fuente.
