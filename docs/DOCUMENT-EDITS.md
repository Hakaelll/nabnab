# Edición de copias PDF

Herramientas incorpora un formulario de edición sobre PDF de Biblioteca. Reutiliza el componente opcional MuPDF 1.28.5 y la cola existente. No instala otro lector ni duplica las operaciones de unir, separar, rotar o extraer texto.

## Recorrido

1. Abre **Editar una copia PDF** y elige un documento importado.
2. Elige la operación y su configuración. Para páginas puedes usar `1-3,5,N`, donde `N` es la última. Una página repetida se procesa una sola vez; la numeración sigue el orden indicado.
3. Elige la página que quieres comparar y pulsa **Previsualizar copia**. Se genera una copia temporal completa y se renderizan el original y el resultado con los mismos límites. Los tamaños mostrados corresponden a esos archivos reales, no a una predicción.
4. Revisa el resultado y los avisos de formularios, firmas y estructura. Confirma la revisión y crea la copia en la cola. Cambiar de archivo o configuración requiere otra previsualización.

La cola produce un archivo nuevo y lo relaciona con el original en Biblioteca. La carpeta de destino es la ya elegida en nabnab. La cancelación de la vista previa termina el proceso nativo; la del trabajo utiliza el mecanismo común de la cola.

## Operaciones

| Operación | Configuración y comportamiento |
| --- | --- |
| Comprimir | Sin pérdida de imagen, equilibrada o menor tamaño. Los perfiles equilibrado y pequeño pueden recomprimir imágenes de color/grises a JPEG de calidad 82/65 y reducirlas a 150/96 DPI si superan 225/144 DPI. Los textos y vectores no se rasterizan. MuPDF elimina objetos sin uso y duplicados y comprime los streams. El resultado puede ocupar más y no se promete un tamaño exacto. |
| Recortar márgenes | Márgenes izquierdo, superior, derecho e inferior en puntos PDF, 72 puntos por pulgada. Cambia el CropBox de las páginas elegidas. Deben quedar al menos 12 puntos visibles por dimensión. El contenido fuera del recorte permanece en el archivo: esta operación no es redacción segura. |
| Numeración visible | Número inicial, prefijo, tamaño y centro superior/inferior. Se añade una capa visible sobre las páginas seleccionadas. Si el texto no cabe, la previsualización falla y pide reducirlo. No modifica las etiquetas lógicas de página ni intenta detectar contenido ocupado. |
| Marca de agua | Texto propio centrado, tamaño y opacidad del 5 al 90 %. No elimina marcas existentes, impide la copia ni afirma derechos sobre el documento. |
| Marcadores / índice | Sustituye el índice interno del PDF con una lista de título, página y nivel de 1 a 4. Permite cargar y editar los marcadores disponibles, agregar entradas, reordenarlas o eliminarlas. La lista vacía elimina el índice de la copia. Máximo 500 entradas y solo destinos internos. No altera los marcadores de Biblioteca. |

Los marcadores con enlaces externos o destinos que no pueden representarse no se convierten en enlaces inventados. Si el índice se carga parcialmente, se muestra un aviso antes de reemplazarlo.

## Originales, permisos y límites

Se exige permiso de edición y se rechazan PDF que requieren contraseña. La entrada está limitada a 1 GiB y 10.000 páginas. No se admite código, comandos, argumentos de motor ni scripts suministrados por el usuario. Las opciones se validan como un tipo cerrado y pasan a un controlador propio como datos JSON.

Numeración y marcas de agua generan sus recursos en un documento de superposición independiente. Se incorporan como Form XObjects, con nombres de recurso únicos y transformaciones de página. Así se conserva el contenido original y se evitan colisiones entre fuentes con el mismo nombre y distintos mapas de caracteres. Los márgenes cambian el área visible; el índice cambia sus objetos de navegación. La compresión usa opciones fijas del `mutool clean` instalado.

La reescritura puede cambiar formularios, anotaciones, metadatos y estructura. La detección de AcroForm y campos de firma añade avisos concretos. No se certifica la validez de una firma en la copia ni la conservación funcional de todos los formularios. El original debe conservarse para comprobar firmas y cualquier propiedad que requiera validez documental.

La salida se escribe en un parcial propiedad del trabajo, se vuelve a abrir para comprobar su número de páginas y se publica sin reemplazar archivos existentes. Una salida incompleta no aparece como resultado terminado en Biblioteca. Las previsualizaciones usan PNG, hasta 1200 × 1600 píxeles y 8 MiB por imagen; el WebView no ejecuta el contenido del PDF.

## Componentes y referencias

La distribución opcional existente usa [MuPDF 1.28.5 de Artifex](https://github.com/ArtifexSoftware/mupdf-downloads/releases/tag/1.28.5), bajo su licencia AGPL-3.0. Esta ampliación no descarga ni activa el componente por sí sola.

Las llamadas se contrastaron con la documentación de [PDFDocument](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/PDFDocument.html), [PDFPage](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/PDFPage.html), [PDFObject](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/PDFObject.html), [DocumentWriter](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/DocumentWriter.html), [Text](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/Text.html) y [mutool clean](https://mupdf.readthedocs.io/en/1.28.5/tools/mutool-clean.html). Los parámetros de compresión también se comprobaron contra la ayuda del ejecutable 1.28.5 instalado para las pruebas.

## Comprobación reproducible

Las pruebas unitarias cubren geometría inválida, opciones desconocidas, jerarquía de marcadores, codificación de Unicode, perfiles de compresión y avisos documentales. La prueba nativa opt-in crea PDF sintéticos locales, aplica todos los perfiles y operaciones, comprueba texto acentuado, páginas giradas, índice jerárquico, previsualización antes/después, integridad byte a byte del original y cancelación previa al arranque.

```powershell
$env:NABNAB_TEST_MUTOOL = (Resolve-Path artifacts/component-verification/mutool.exe).Path
cargo test --manifest-path src-tauri/Cargo.toml document_edits::tests -- --include-ignored --test-threads=1
```

La ruta del ejemplo es el componente ya preparado para verificación; en otra máquina debe indicarse el `mutool.exe` instalado y autorizado. Los documentos sintéticos no prueban la conservación de firmas certificadas, formularios comerciales o todos los PDF dañados. Revisa la copia real antes de utilizarla para esos fines.
