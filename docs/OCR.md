# OCR local

Herramientas > Documentos > Reconocer texto admite una imagen de Biblioteca o una selección de páginas de un PDF. Los originales se conservan. El trabajo usa la cola, el progreso, la cancelación y los resultados relacionados de nabnab.

## Componentes e idiomas

Instala y activa OCR en Ajustes > Componentes. El componente es Tesseract 5.5.4 para Windows x64, con sus DLL de ejecución app-local procedentes de paquetes Cygwin fijados. La descarga suma 6.678.346 bytes; reserva unos 40 MB para archivos instalados y temporales. No instala Cygwin, shells, servicios, utilidades de mantenimiento ni modifica PATH o el registro. Los paquetes se comprueban con SHA-512 y solo se extraen Tesseract, sus DLL, la fuente de PDF y avisos de licencia. El resto de ejecutables se excluye. Un inventario SHA-256 comprueba los archivos antes de usarlos.

Los idiomas se descargan aparte, con un diálogo que muestra tamaño y licencia:

- Español, `spa`: 2.294.433 bytes.
- Inglés, `eng`: 4.113.088 bytes.

Son modelos `tessdata_fast` oficiales con licencia Apache-2.0, fijados al commit `87416418657359cb625c412a48b6e1d6d41c29bd`. La instalación no incluye modelos por defecto. Una vez instalados, el reconocimiento funciona sin conexión. No hay servicio remoto ni fallback en la nube.

Los paquetes y sus fuentes se enumeran en `src-tauri/src/ocr/packages.json`. Se elige Tesseract 5.5.4 porque esa versión corrige problemas de seguridad de versiones anteriores. Los instaladores Windows conocidos y algunos paquetes NuGet consultados ofrecían versiones antiguas. Esta distribución utiliza el motor actualizado sin ejecutar un instalador del sistema. Las dependencias tienen licencias Apache, BSD, MIT, LGPL y GPL con las excepciones de ejecución que correspondan; el componente conserva sus avisos y referencias a los paquetes fuente.

## Imágenes y regiones

Selecciona una imagen, uno o ambos idiomas y texto como resultado. Para una región, introduce X, Y, ancho y alto en píxeles del original, con origen en la esquina superior izquierda. El servicio comprueba las dimensiones antes de recortar. Las imágenes animadas usan el primer fotograma. La entrada se decodifica localmente con FFmpeg y se limita a 40 millones de píxeles.

El resultado de texto puede revisarse en Biblioteca. El índice conserva página, texto, palabras, confianza y coordenadas del motor. En una región, las coordenadas son relativas al recorte y se conserva la posición del recorte en la imagen original. OCR puede equivocarse en cifras, nombres y acentos. No se promete reconstrucción de tablas ni reconocimiento fiable de manuscritos.

## PDF buscable

El OCR de PDF requiere también el componente opcional Documentos, MuPDF 1.28.5. Selecciona páginas como `1-3,5,N`, una resolución de 150 a 300 DPI y texto o copia PDF buscable. Hay un máximo de 200 páginas por trabajo.

Cada página se procesa por separado. Se extrae primero su texto; una página con al menos doce caracteres alfanuméricos y dos palabras se considera provista de texto útil y conserva esa capa, sin aplicar OCR de nuevo. Es una heurística explícita, no un diagnóstico perfecto de la calidad del documento. Un número de página aislado no evita reconocer un escaneo.

Las páginas que requieren OCR se renderizan con límites de tamaño y se reconocen mediante Tesseract. Para el PDF buscable, Tesseract produce una capa de texto invisible con su fuente y mapeo Unicode. MuPDF la incorpora como un Form XObject al contenido original, conservando sus recursos y ajustando las transformaciones de página. No se sustituye la página original por una imagen rasterizada. La copia se guarda en un archivo nuevo y mantiene las páginas no seleccionadas.

Antes de generar la copia debes aceptar que la estructura, formularios o firmas pueden cambiar. No se garantiza la validez de firmas en el resultado. Se respetan las restricciones de extracción y representación del documento; no se quitan contraseñas ni se eluden permisos.

## Buscar y corregir

El texto reconocido se guarda en un índice local separado, con una referencia pequeña en Biblioteca. El índice no se incluye completo en las instantáneas de la aplicación y se limita a 16 MiB. Incluye la huella SHA-256 de la versión del origen. El origen se vuelve a comprobar al finalizar el reconocimiento para evitar asociar el índice a un archivo que cambió durante el trabajo.

La búsqueda distingue texto original y OCR. Una frase reconocida puede señalar las palabras correspondientes en la vista previa cuando el motor entregó coordenadas. Las correcciones se aplican al índice generado, no al archivo original ni a la capa de texto del PDF. Al corregir una página se retiran sus coordenadas anteriores para no conservar posiciones inexactas. También se puede retirar únicamente la parte OCR del índice, conservando el texto original. Guardar el texto corregido crea otro resultado relacionado en la cola.

## Verificación

Las pruebas unitarias cubren TSV, coordenadas, confianza, búsqueda de frases, límites, rutas de índice, distinción entre original y OCR y corrección o retirada del índice. Los recorridos con motores reales son opt-in:

```powershell
$env:NABNAB_TEST_OCR_PACKAGES = '<carpeta de los 21 paquetes fijados>'
cargo test --manifest-path src-tauri/Cargo.toml real_component_installs_and_verifies_app_local -- --ignored --nocapture
```

```powershell
$env:NABNAB_TEST_TESSERACT = '<ruta absoluta a tesseract.exe verificado>'
$env:NABNAB_TEST_TESSDATA = '<carpeta de eng/spa.traineddata y pdf.ttf>'
$env:NABNAB_TEST_MUTOOL = '<ruta absoluta a mutool.exe 1.28.5>'
$env:NABNAB_TEST_FFMPEG = '<ruta absoluta a ffmpeg.exe de nabnab>'
cargo test --manifest-path src-tauri/Cargo.toml real_ocr_preserves_pdf_appearance_original_text_and_source -- --ignored --nocapture
```

El 6 de noviembre de 2026 pasaron las seis pruebas unitarias OCR, la instalación real del componente desde sus 21 paquetes verificados y el recorrido completo con motores locales. La prueba de instalación ejecutó Tesseract 5.5.4, comprobó el rechazo de un archivo manipulado y retiró el componente conservando la carpeta separada de modelos.

La prueba real de OCR creó un PDF con una página de texto y otra escaneada, reconoció la segunda y comprobó extracción buscable, integridad del origen y un PNG renderizado idéntico antes y después. También probó una región de imagen y cancelación previa al procesamiento. Se ejecutaron con acceso nativo explícito porque el sandbox bloquea la inicialización de objetos de ejecución de Cygwin y el reemplazo de índices temporales de Windows. No se modificó una instalación del usuario ni se enviaron documentos a la red. Los fixtures propios no certifican la calidad sobre todos los documentos o manuscritos. La interacción final de la interfaz y la publicación por cola se comprueban por separado durante la integración.

## Referencias oficiales

- [Tesseract 5.5.4 y notas de versión](https://tesseract-ocr.github.io/tessdoc/ReleaseNotes).
- [Tesseract, parámetros de línea de comandos](https://tesseract-ocr.github.io/tessdoc/Command-Line-Usage.html).
- [Cygwin, paquetes Tesseract](https://cygwin.com/packages/summary/tesseract-ocr.html) y [licencias](https://cygwin.com/licensing.html).
- [Modelos tessdata_fast](https://github.com/tesseract-ocr/tessdata_fast).
- [MuPDF PDFPage](https://mupdf.readthedocs.io/en/latest/reference/javascript/types/PDFPage.html) y [PDFDocument](https://mupdf.readthedocs.io/en/latest/reference/javascript/types/PDFDocument.html).
