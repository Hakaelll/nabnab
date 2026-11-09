# Enviar a un dispositivo

Nabnab usa una instalación existente de KDE Connect para Windows. Empareja el teléfono y activa el complemento de compartir archivos en KDE Connect. Desde **Enviar a dispositivo** puedes actualizar la lista o elegir `kdeconnect-cli.exe` para la sesión actual si nabnab no lo encuentra. La detección busca el ejecutable en `PATH` y en ubicaciones habituales del instalador. [Descarga y emparejamiento de KDE Connect](https://kdeconnect.kde.org/download.html)

1. Abre **Enviar a dispositivo** en un archivo, una selección de Biblioteca, una colección o el resultado de un trabajo en Descargas. En **Ampliar imagen**, **Eliminar fondo** y **Stickers**, busca **Última exportación** debajo del formulario. Ahí aparecen los archivos publicados del último trabajo o lote, con su tamaño y su botón de envío. **Separar pistas** permite enviar los FLAC juntos y **Traducir subtítulos** ofrece el subtítulo, el WAV y el MKV por separado. También puedes abrir el diálogo desde la cabecera de Biblioteca o elegir texto o un enlace público en el lector.
2. Elige un dispositivo emparejado y accesible. Pulsa **Revisar envío** para ver el contenido y el total de bytes. Los archivos se leen desde sus ubicaciones en Biblioteca. Para empaquetar una selección, pulsa **Preparar ZIP de la selección** y revisa su contenido antes de crearlo. Las notas, los metadatos opcionales y el OCR vienen desmarcados. Cuando termine la exportación, **Revisar envío del ZIP** aparece en Colección portátil con el nombre y tamaño reales del archivo. También puedes abrir el trabajo en Descargas.
3. Confirma el envío. Si un archivo cambia o el dispositivo deja de estar accesible durante la revisión, vuelve a comprobar la selección. Cada revisión permite una sola confirmación y caduca tras cinco minutos.

Si un archivo de la selección falta en el disco, aparece por separado como no disponible. Pulsa **Quitar del envío** para revisar o empaquetar los archivos restantes. La entrada del archivo ausente sigue en Biblioteca.

![Selección con un archivo disponible y otro ausente](device-share-missing.png)

Al abrir **Enviar a dispositivo** desde un archivo de arXiv, Zenodo o LibriVox, nabnab empieza con la copia local. Si conserva la ficha pública del proveedor, pulsa **Usar enlace de origen** para pasar al enlace. La revisión muestra la URL completa y el dispositivo elegido. El enlace de arXiv conserva la versión del PDF y el de LibriVox abre la ficha del audiolibro cuando el catálogo la incluye. Los enlaces de sesión o con firma quedan fuera de esta opción. [Campos del catálogo de LibriVox](https://librivox.org/api/info)

![Opción de compartir la ficha de un artículo de prueba desde Biblioteca](device-share-source.png)

Puedes enviar hasta 50 archivos por solicitud, un enlace público o hasta 8 KiB de texto. El contador mide bytes UTF-8 y **Revisar envío** se habilita cuando el texto cabe. Los enlaces con credenciales, fragmentos, destinos locales o parámetros de acceso se rechazan también cuando aparecen en el texto. Se admite el parámetro público `v` de los enlaces de vídeo de YouTube. Para enlaces privados puedes elegir la copia local.

En el lector de PDF y EPUB, abre **Texto de esta página**, selecciona un fragmento y pulsa **Enviar texto seleccionado**. La revisión muestra el nombre del documento, la página y el texto que se va a enviar. El lector comprueba los permisos de copia antes de ofrecer la acción. [Lector de documentos](DOCUMENTS.md#lector-pdf-y-epub)

![Revisión del fragmento seleccionado de un EPUB sintético en Windows](device-share-document-excerpt.png)

**Entregado a KDE Connect** indica que la CLI respondió sin errores al pasarle la solicitud. Consulta KDE Connect para ver el progreso o confirmar la recepción. **Entrega a KDE Connect sin confirmar** distingue un error de la CLI, una respuesta vacía o ilegible y un número de respuestas distinto del número de archivos revisados. El recuento corresponde a la solicitud entera. El resultado conserva el destino y los archivos revisados para que puedas comprobarlos en KDE Connect antes de iniciar otro envío. [Comandos de la CLI de KDE Connect 26.08.2](https://github.com/KDE/kdeconnect-kde/blob/v26.08.2/cli/kdeconnect-cli.cpp)

![Resultado de un envío de dos archivos con una respuesta parcial de la CLI de prueba](device-share-unconfirmed.png)

La prueba de Windows utiliza un ejecutable de prueba que registra argumentos en un perfil aislado. Verifica la revisión de tamaños, los nombres Unicode, el rechazo de enlaces firmados y el envío desde el resultado de un ZIP portátil real. También comprueba que un archivo modificado, un dispositivo desconectado o una segunda confirmación impiden ejecutar la solicitud. Si la CLI devuelve una sola línea para dos archivos, la interfaz conserva ambos nombres y el destino con una sola invocación de envío registrada. La prueba del lector verifica el texto seleccionado mediante IPC simulado. La entrega a un teléfono físico queda pendiente de comprobar. [Revisión de un envío en Windows](device-share-native.png) · [ZIP listo para revisar](device-share-portable.png) · [Copia doblada lista para enviar](dubbing-share.png)

La prueba de Windows también genera un sticker WebP, verifica que reaparece en la herramienta tras recargar y confirma que la CLI de prueba recibe únicamente ese archivo. Los casos de ampliación y recorte usan resultados de prueba para verificar el alcance del lote y la exclusión de archivos ausentes o ajenos. [Sticker publicado en Herramientas](sticker-export-share.png)

Con entradas de prueba en Biblioteca, la misma prueba comprueba la selección por teclado, el foco en el enlace y la revisión a 900 y 1180 px. La CLI recibe exactamente la URL pública de cada uno de los tres catálogos. Para los dos casos con procedencia privada o firmada, solo se prepara la copia local.
