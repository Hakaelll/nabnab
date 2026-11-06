# Organización de archivos elegidos

Herramientas → Archivos reúne duplicados exactos, imágenes parecidas y renombrado. La entrada son registros elegidos de Biblioteca; no se recorren discos ni carpetas no seleccionadas. Se conserva la dirección visual de [design-reference.md](design-reference.md), con la selección junto a la configuración y resultados debajo.

## Duplicados y revisión

El análisis exacto agrupa primero por el tamaño real del archivo. Solo lee el contenido de los grupos candidatos, por bloques de 64 KiB, y confirma las coincidencias con SHA-256. Dos registros de la misma ubicación son relaciones compartidas; no se presentan como dos copias físicas. El resultado muestra ubicación, colecciones y registros que comparten el archivo.

Las imágenes parecidas usan FFmpeg y FFprobe ya presentes. Se consulta el tamaño, se limita cada imagen a 40 millones de píxeles y se genera una muestra local de 9 × 8 píxeles en gris. Las 64 comparaciones entre píxeles contiguos forman un dHash. La distancia es el número de bits distintos: no es un porcentaje de certeza ni una medida de calidad. Las parejas deben tener proporciones que difieran como máximo un 5 %. Las imágenes animadas usan su primer fotograma.

Se admiten hasta 200 registros por análisis, es decir, como máximo 19.900 parejas visuales. La interfaz muestra veinte parejas por página y permite compararlas lado a lado. Una huella coincidente puede corresponder a imágenes diferentes: no se llaman duplicados exactos ni se eliminan automáticamente.

La acción de revisión añade los registros marcados a una colección elegida. Conserva el archivo, sus otras colecciones y los registros no seleccionados. No hay borrado físico en estas herramientas. Guardar un informe utiliza la cola común y crea un resultado nuevo; un informe contiene ubicaciones locales y debe revisarse antes de compartirlo.

## Renombrado

Selecciona hasta 200 registros y configura una plantilla. Se conserva el orden de selección, se agrupan los registros de una misma ubicación y se muestra el mapa completo antes de confirmar:

| Token | Valor |
| --- | --- |
| `{name}` | Nombre sin extensión, después de las sustituciones |
| `{ext}` | Extensión sin el punto |
| `{extension}` | Extensión con el punto, o texto vacío si no existe |
| `{counter}` | Contador inicial más posición, con el mínimo de dígitos elegido |
| `{date}` | Fecha que se elige explícitamente en el formulario |

La sustitución se aplica al nombre sin extensión. Puede ser literal o una expresión de Rust regex: hasta 512 bytes de patrón y de sustitución, con presupuesto de compilación de 64 KiB. Las capturas admiten `$1` y `${nombre}`. No se admiten backreferences ni look-around. El resultado se valida como un único componente de nombre, sin rutas, nombres reservados de Windows o sobrescrituras. Las colisiones entre resultados también se rechazan. Para cambiar solo mayúsculas y minúsculas, usa primero un nombre intermedio distinto.

La previsualización caduca a los diez minutos. Encolar una previsualización válida crea su operación autorizada persistente; la espera en la cola no consume otra caducidad. Antes de ejecutarla se vuelve a comprobar el contenido. En Windows se mantienen abiertos los archivos verificados con lectura y permiso de renombrado, sin compartir escritura. El cambio utiliza el identificador abierto del archivo y no reemplaza un destino que aparezca después. Los trabajos de renombrado y deshacer requieren la barrera exclusiva de la cola para no competir con otros trabajos de la aplicación.

Los identificadores de Biblioteca permanecen estables. Se actualizan todos los registros de la misma ubicación, las rutas de pistas relacionadas y las salidas de trabajos terminados. Las relaciones por identificador, colecciones, etiquetas, anotaciones y referencias OCR permanecen asociadas al mismo registro.

## Historial, interrupción y deshacer

Los planes y operaciones se guardan bajo `file-operations` en la configuración local. La operación registra cada paso antes de moverlo y conserva el mapa anterior/nuevo, SHA-256, tamaño y estado. Disco y persistencia de Biblioteca son pasos separados: no se promete una transacción atómica entre ambos.

Si una operación se interrumpe, su estado y error quedan visibles. Continuar verifica los archivos de nuevo y adopta un movimiento ya realizado únicamente cuando su contenido coincide con el plan. No ejecuta dos veces el mismo mapa. Cancelar conserva los pasos completados para revisión o deshacer explícito.

Deshacer verifica que el contenido siga siendo el del archivo renombrado y que el nombre anterior esté libre. Si otro archivo ocupa ese nombre, se detiene y lo conserva. Una modificación del contenido también requiere revisión manual. El historial mantiene hasta cincuenta operaciones en la vista, sin borrar los registros persistentes anteriores.

## Contratos de integración

`analyze` recibe una copia de Biblioteca, identificadores seleccionados, directorio de trabajo y cancelación. Los hashes se leen fuera del bloqueo de persistencia. `plan_rename` también trabaja sobre una copia, admite cancelación y no cambia archivos.

`approve_rename` comprueba el plan y crea la operación en cola sin releer el contenido bajo el bloqueo. `preflight_rename` verifica los hashes fuera del bloqueo y devuelve `VerifiedOperation`, un valor interno que mantiene los archivos abiertos y no puede deserializarse desde RPC. `execute_rename` o `undo_rename` consumen ese valor y realizan los pasos de disco/persistencia bajo el propietario único del repositorio. El host comprueba `journal.successful()` para distinguir resultados completos de parciales.

La interfaz usa `file_analysis`, `queue_file_analysis`, `file_rename_preview`, `queue_file_rename`, `file_rename_history`, `queue_file_undo`, `file_organize_review` y `cancel_file_operation`. El cliente transmite identificadores, opciones y referencias de planes; no introduce rutas arbitrarias.

## Verificación

Las pruebas locales cubren contenido exacto frente a igual tamaño, registros compartidos, renombrado sin cambiar contenido, identificadores estables, pistas y salidas de trabajos, colisión aparecida después de verificar, historial parcial durable, recuperación del movimiento interrumpido, deshacer sin sobrescribir, expresiones acotadas y exclusión de escritores mientras se conservan los identificadores abiertos.

La prueba opcional `real_image_similarity_matches_a_resized_copy_without_changing_originals` usa el FFmpeg verificado indicado mediante `NABNAB_TEST_FFMPEG`: crea una imagen y una copia reducida, comprueba el parecido y conserva los bytes del original. Las pruebas renderizadas de la interfaz utilizan fixtures de IPC y no acreditan cambios reales en el sistema de archivos.

El 6 de noviembre de 2026 pasaron seis pruebas Rust de organización, la prueba real de dHash con FFmpeg y la prueba de la barrera exclusiva de la cola. La interfaz pasó `tests/file-organization.browser.mjs` en 1180 × 780 y 900 × 620: selección, ubicaciones compartidas, colección de revisión, informe, comparación visual, colisión de nombres, expresión regular, encolado, deshacer, cancelación y foco de teclado. Las capturas y `verification.json` están en `artifacts/file-organization`; son evidencia renderizada con fixtures, no pruebas de renombrado desde la interfaz nativa. También pasó TypeScript con `node node_modules/typescript/bin/tsc --noEmit`.

El recorrido integrado con la aplicación de escritorio, cola y biblioteca reales también pasó con archivos sintéticos: [evidencia nativa de la entrega C](verification/asset-workflows-native.json). Las capturas nativas están en [la guía integrada](ONLINE-LOCAL.md).
