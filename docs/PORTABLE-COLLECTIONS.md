# Colecciones portables

Herramientas → Archivos y colecciones → Colección portable permite crear un ZIP con archivos elegidos de Biblioteca. La revisión calcula SHA-256, tamaño y contenido repetido antes de encolar la copia. Un archivo no se elimina ni se sobrescribe.

El ZIP contiene `manifest.json`, archivos con referencias relativas y, cuando corresponde, la captura local validada de los artículos. Se pueden incluir colecciones, etiquetas, procedencia, marcadores, subtítulos y notas. El índice OCR requiere activar su opción. No se exportan sesiones, cookies, tokens de nabnab, miniaturas de red ni rutas absolutas de la biblioteca. Los enlaces de procedencia pierden consultas y fragmentos. El contenido de los archivos elegidos se copia íntegro: debe revisarse también antes de compartirlo.

Los artículos incluyen su documento de bloques e imágenes PNG locales. Sus notas conservan la relación con esa captura, no con otra captura de la misma URL. Si sanear la URL cambia la referencia del documento, se vuelve a calcular la referencia sin cambiar sus bloques y solo se conservan las notas cuyo texto y contexto coinciden.

## Importar

Elige un ZIP portable o su carpeta ya extraída y pulsa Revisar colección. La revisión comprueba esquema, nombres relativos, tamaños, SHA-256 y duplicados. La importación utiliza el destino autorizado de nabnab y crea una subcarpeta nueva. La revisión caduca en 24 horas y está vinculada a origen, contenido y destino; cambiar el destino requiere una revisión nueva.

Los archivos ordinarios idénticos pueden reutilizar un registro de Biblioteca y combinar sus colecciones, etiquetas y marcadores. Las relaciones padre/resultado se reconstruyen con los identificadores locales nuevos. Artículos, resultados OCR y registros con relaciones distintas conservan copias separadas cuando reutilizarlos trasladaría metadatos a otra versión.

Una importación de artículo verifica primero los bytes del paquete y después regenera HTML, EPUB o Markdown a partir de su captura validada. No confía en que un HTML arbitrario sea seguro porque el manifiesto lo llame artículo. El índice OCR se valida antes de restaurarlo. Los archivos ejecutables se guardan como archivos y nunca se ejecutan.

El ZIP se inspecciona con el lector ZIP de Windows mediante un script fijo. No se interpolan nombres del paquete como código. Se rechazan traversal, enlaces simbólicos, nombres reservados o colisiones sin distinguir mayúsculas. Los límites son 2000 registros, 10000 archivos, 128 GiB de contenido y 16 MiB de manifiesto. Una entrada ZIP no puede superar una expansión de 1000 veces su tamaño comprimido más 1 MiB. El manifiesto se limita por los bytes realmente leídos antes de almacenarlo o decodificarlo. Los temporales se crean en un espacio privado fuera de los nombres del paquete. El lector de Windows puede rechazar rutas demasiado largas; en ese caso usa una ubicación más corta o importa la carpeta ya extraída.

## Cola, cancelación y recuperación

La exportación y la importación usan Descargas. Cancelar interrumpe el procesamiento; un parcial no se registra como resultado completo. Una importación interrumpida puede conservar archivos verificados en su carpeta administrada para reintentar. Un archivo distinto que aparezca en un destino previsto causa un error y no se sobrescribe.

La publicación de archivos y la actualización del índice no forman una transacción atómica del sistema de archivos y la base de datos. El registro se confirma después de verificar las copias. Reintentar una confirmación ya aplicada conserva sus identificadores y no añade otra copia.

El escritor de paquetes comparte `portable_zip.rs` con las herramientas de imágenes. Escribe ZIP almacenado sin compresión y utiliza ZIP64 cuando los tamaños u offsets lo necesitan. Lee archivos por bloques, calcula CRC-32 y comprueba que no cambien durante la copia. No carga todo el contenido en memoria.

Esto transporta la selección y sus metadatos compatibles. No es una copia de seguridad completa de nabnab ni del ordenador.

## Verificación

`cargo test --manifest-path src-tauri/Cargo.toml portable -- --nocapture` comprueba nombres inseguros, integridad de archivos, aprobación vinculada al destino, deduplicación, relaciones padre/resultado, confirmación idempotente, OCR y notas. También comprueba que un HTML malicioso se regenere desde su captura segura, que una captura o un OCR inválidos se rechacen en la revisión y que la expansión ZIP se limite por los bytes realmente leídos. El recorrido de ZIP usa el lector real de Windows. ZIP64 tiene comprobaciones de cabeceras sintéticas; no se afirma una copia real de más de 4 GiB.

`node tests/portable-collections.browser.mjs`, con Vite y Playwright disponibles, verifica selección con teclado, revisión de duplicados, opciones, consentimiento separado, invalidación y cancelación en ventanas de 1180 × 780 y 900 × 620. Sus capturas e informe usan IPC simulado y no certifican una importación nativa.

El recorrido integrado con la aplicación de escritorio, cola y biblioteca reales también pasó con archivos sintéticos: [evidencia nativa de la entrega C](verification/asset-workflows-native.json). Las capturas nativas están en [la guía integrada](ONLINE-LOCAL.md).
