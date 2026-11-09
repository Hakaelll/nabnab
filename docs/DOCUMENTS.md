# Documentos locales

El módulo usa un único componente opcional, MuPDF `mutool`, para PDF y EPUB. No añade Python, un navegador PDF embebido ni otro runtime de aplicaciones. Las operaciones generan trabajos en la cola y archivos nuevos en la biblioteca.

## Operaciones PDF

| Operación | Resultado | Límites |
| --- | --- | --- |
| Unir | PDF con los documentos en el orden elegido | De 1 a 100 entradas, sin sustituir originales. |
| Seleccionar, separar o reordenar páginas | Un PDF con las páginas indicadas | `1-3,5,N` selecciona páginas; `N` es la última; `5-1` invierte el orden. Seleccionar una página permite separarla del documento. |
| Rotar | Copia PDF reconstruida como contenido vectorial | 90°, 180° o 270°. Reconstruir puede cambiar anotaciones, formularios y metadatos; no conserva firmas digitales. |
| Extraer texto | TXT con texto disponible | No hace OCR. Un escaneo sin texto devuelve un error, no un éxito vacío. |
| Exportar página | PNG | Una página por tarea, de 72 a 300 DPI, con un máximo de 8192 píxeles por dimensión. |

El parser limita documentos y selecciones a 10.000 páginas. Comprueba rangos antes de ejecutar el motor. Los PDF que necesitan una contraseña no se abren automáticamente. Las operaciones comprueban permisos de copia, impresión o ensamblado, según corresponda.

## Lector PDF y EPUB

El lector recibe una imagen PNG renderizada y texto plano. No introduce PDF, EPUB, HTML del libro ni JavaScript del documento en el WebView. Los enlaces del documento no son controles interactivos y no se abren automáticamente.

Permite avanzar y retroceder, conservar la página actual, marcar páginas y buscar en el texto disponible. La posición y los marcadores usan la persistencia de biblioteca existente. EPUB utiliza un tamaño de página y fuente fijos para mantener la correspondencia entre la página guardada y la lectura posterior. La búsqueda devuelve hasta 100 páginas con coincidencias; si el motor no conserva límites de página, informa de esa limitación en vez de asignar resultados a páginas inventadas.

Abre **Texto de esta página** y marca un fragmento con el puntero o con **Mayús + flechas**. Pulsa **Enviar texto seleccionado** para revisar el fragmento y el dispositivo en KDE Connect. Puedes editar el texto en el diálogo o elegir la copia local desde **Archivos**. El lector muestra un aviso cuando no puede ofrecer texto para copiar. [Envío a un dispositivo](DEVICE-SHARE.md)

La vista limita la imagen a 1400 × 2000 píxeles y 8 MiB. El texto de cada página se limita a un millón de caracteres. El lector no ofrece OCR ni desbloqueo de libros protegidos.

## Procedencia y comprobación

La implementación sigue la documentación oficial de [mutool merge](https://mupdf.readthedocs.io/en/1.28.5/tools/mutool-merge.html), [mutool draw](https://mupdf.readthedocs.io/en/1.28.5/tools/mutool-draw.html) y la [API Document](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/Document.html). Un controlador de inspección escrito por nabnab obtiene páginas y permisos; el nombre del archivo se pasa como argumento y nunca se interpola en código.

El componente Windows procede de [ArtifexSoftware/mupdf-downloads 1.28.5](https://github.com/ArtifexSoftware/mupdf-downloads/releases/tag/1.28.5), con licencia AGPL-3.0. La descarga y activación son voluntarias. Las herramientas usan únicamente el ejecutable gestionado por Componentes.

Las pruebas unitarias cubren selección y orden de páginas, entradas inválidas, separación de argumentos y búsqueda por página. Con MuPDF verificado en un perfil aislado, WebView2 abrió un PDF y un EPUB sintéticos y preparó solo el texto marcado. Un PDF con la copia deshabilitada mostró la página y mantuvo la acción de envío fuera del lector. [Revisión de un fragmento del EPUB](device-share-document-excerpt.png)
