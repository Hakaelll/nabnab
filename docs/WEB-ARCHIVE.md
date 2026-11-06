# Guardar artículos y páginas offline

Reinstala o recarga la extensión Chrome/Edge desde `browser-extension/` y conserva su emparejamiento con nabnab. No se añaden permisos de navegación global ni acceso a cookies.

1. Abre la pestaña que quieres conservar y pulsa la extensión.
2. Elige Guardar artículo, Guardar selección o Guardar página offline. Las mismas acciones están en el menú contextual y abren una previsualización.
3. Revisa título, autor disponible, vínculo de origen, fecha de archivo, contenido y avisos. Elige colección, etiquetas y HTML, EPUB o Markdown.
4. Confirma la copia. La extensión espera un recibo de la aplicación; un fallo permite reintentar la misma solicitud.
5. Consulta Descargas y abre el resultado en Biblioteca. El lector permite seleccionar texto y guardar un resaltado o una nota.

La selección se limita a un bloque para añadir notas. Su texto exacto, contexto y versión quedan vinculados a esa copia local. Una captura nueva de la misma URL genera otra versión; las notas no se trasladan silenciosamente. También se pueden crear notas con teclado desde el botón de cada bloque. Escape cierra el lector y devuelve el foco, sin cerrar el inspector que pudiera estar debajo.

## Qué conserva cada modo

Artículo busca `main`, un elemento con `role=main` o `article` y conserva texto visible, encabezados, listas, citas, código e imágenes copiables. Utiliza una extracción local acotada sobre el DOM renderizado, sin Readability ni un servicio de extracción remoto. Puede ser necesario usar Guardar selección en páginas sin una estructura clara.

Página conserva esos elementos en su orden y simplifica la presentación. No conserva la distribución exacta, controles interactivos, vídeos, fuentes, hojas de estilo ni aplicaciones web completas. No sigue enlaces, subpáginas o adjuntos. Los avisos identifican recursos omitidos o límites alcanzados.

Las imágenes se copian desde elementos ya cargados mediante las reglas de acceso del navegador. No se realizan nuevas solicitudes. Una imagen que no permite copia por CORS queda indicada como ausente. Se guardan hasta 32 PNG locales, con dimensión máxima de 2048 px y un presupuesto de recursos de 2 MB. El recorrido del DOM, el texto y la solicitud también tienen límites explícitos.

## Contenido y privacidad

La captura excluye formularios, controles, campos ocultos, contenido editable, scripts, iframes y objetos. Una selección que atraviesa un formulario o editor se rechaza. No se consulta ni exporta el almacenamiento del navegador, cookies, contraseñas o perfiles. El modo de edición de página `designMode` no se captura.

El protocolo transporta bloques tipados y PNG, no HTML del sitio. La aplicación valida el contenido de nuevo y genera los archivos con texto escapado. HTML incluye una política que deshabilita scripts y solicitudes externas. EPUB empaqueta sus imágenes; Markdown solo genera imágenes locales y conserva como texto literal los ejemplos de sintaxis del artículo. La lectura dentro de nabnab también usa bloques escapados y recursos locales.

La URL de procedencia omite parámetros de acceso reconocidos. La fecha de captura se conserva separada de la fecha de publicación disponible. El archivo guardado en una página autenticada contiene únicamente el contenido que elegiste capturar; no se intenta eludir restricciones de acceso o descarga.

## Comprobación

- `tests/archive-capture.browser.mjs` ejecuta la extracción sobre DOM real en Edge y comprueba editores, selecciones que cruzan formularios, contexto, contenido oculto y límites.
- `tests/articles.browser.mjs` verifica el lector con IPC simulado en 1180 × 780 y 900 × 620: texto y notas escapados, selección Unicode, citas repetidas, versiones, foco, exportación y ausencia de peticiones remotas.
- `web_archive::tests` comprueba validación, archivos nuevos, inmutabilidad, cancelación y formatos. `real_epub_contains_readable_archived_text` abre el EPUB generado con MuPDF real sin red.

El protocolo nuevo usa `/archive` en el puente local emparejado existente. Mantiene origen exacto, autenticación y recibos. El límite de 4 MiB de captura no amplía el límite anterior del endpoint de descargas.
