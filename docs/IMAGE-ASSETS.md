# Capturas, iconos y sprites

Herramientas > Capturas, iconos y atlas reúne tres recorridos de producción y la separación de cuadrículas. Trabajan sobre PNG o JPEG de Biblioteca, en el orden que elijas. Reutilizan FFmpeg gestionado por nabnab; no añaden otro motor obligatorio, instalan software ni examinan el disco.

## Uso común

1. Importa las imágenes o búscalas por nombre en la selección de Biblioteca. Los botones Subir y Bajar cambian el orden utilizado para unir o empaquetar.
2. Elige la operación y sus opciones. Revisar dimensiones consulta los archivos reales. En Unir capturas, Buscar solapamientos también prepara sugerencias para los cortes.
3. Previsualiza el resultado. La imagen procede de los archivos generados localmente. Cambiar entradas, orden o configuración invalida la vista anterior.
4. Crea el resultado en la cola. El destino es la carpeta elegida en nabnab; el archivo nuevo aparece en Biblioteca. La cancelación usa el mismo mecanismo que los otros trabajos y conserva las entradas.

Los PNG conservan alfa. La cuadrícula de fondo de la previsualización representa transparencia y no se añade a los archivos.

## Unir capturas verticales

Selecciona de 2 a 32 imágenes con la misma anchura. La primera permanece completa. Para cada imagen posterior, el corte indica cuántas filas se quitan por arriba antes de colocarla al final de la anterior.

La detección compara filas muestreadas de cada pareja, con un máximo de 4096 píxeles de solapamiento. Muestra un corte sugerido, su coincidencia estimada y si el resultado es ambiguo. Probar un corte es una acción explícita: las sugerencias no se aplican silenciosamente.

Las zonas planas, las cabeceras repetidas y el contenido animado pueden producir coincidencias engañosas. Corrige los cortes y revisa las uniones en la vista previa. Si las anchuras difieren, prepara copias recortadas o redimensionadas con las herramientas existentes.

El resultado es `joined-screenshots.png`, sin recodificar los píxeles que se conservan. No se promete reconstruir una página web ni recuperar contenido que no esté en las capturas elegidas.

## Paquete de iconos

Desde una imagen puedes elegir PNG de 16, 24, 32, 48, 64, 96, 128, 192, 256, 512 y 1024 píxeles. Debes seleccionar al menos un tamaño de hasta 256 para el ICO.

El encuadre permite conservar la proporción y añadir espacio transparente o recortar el centro para llenar el cuadrado. El fondo se elige por separado: conservar alfa o componer toda la imagen sobre un color `#RRGGBB`. Componer sobre un color también rellena las zonas transparentes del interior, no solo los márgenes.

`icon-package.zip` incluye los PNG, `favicon.ico` y `manifest.json`. El ICO contiene imágenes PNG de los tamaños seleccionados hasta 256; los PNG mayores permanecen como archivos independientes. El manifiesto registra tamaños, encuadre, fondo y codificación. Debe comprobarse en los navegadores o aplicaciones de destino; no se anuncia compatibilidad universal.

El redimensionado usa Lanczos. La interpolación puede redondear canales, incluido alfa; conservar transparencia no implica que una imagen redimensionada mantenga todos sus valores originales de píxel.

## Separar una cuadrícula

Indica columnas, filas y espacio entre celdas. Esta cuadrícula no tiene margen exterior y sus dimensiones deben dividir exactamente la imagen, una vez descontados los espacios. Se admiten hasta 256 fotogramas.

Puedes recorrerla por filas o por columnas. `grid-frames.zip` contiene `frame-0000.png`, los demás fotogramas y un manifiesto con el rectángulo de origen de cada uno. No se escala ni se elimina el alfa de los recortes.

La vista previa muestra un mosaico de los primeros 16 fotogramas como máximo. El mensaje indica cuántos se muestran y cuántos contiene el paquete. Los archivos del paquete conservan sus dimensiones reales; las miniaturas de comparación no son esos archivos.

## Crear un atlas

Selecciona las imágenes en el orden de los fotogramas y configura columnas, tamaño de celda, padding y encuadre. El padding se aplica entre celdas y también al borde exterior. Las celdas y el espacio libre conservan transparencia.

`sprite-atlas.zip` incluye `atlas.png` y `atlas.json`. El JSON declara el tamaño del atlas, las celdas y una lista ordenada de rectángulos `x`, `y`, `width`, `height`, medidos desde la esquina superior izquierda. No rota ni recorta automáticamente los márgenes transparentes de los fotogramas. Es un formato explícito de nabnab que puede adaptarse al motor de destino; no se atribuye compatibilidad con motores que no se han comprobado.

## Límites y privacidad

- Entradas PNG/JPEG de hasta 64 MiB y 24 millones de píxeles cada una, con un máximo de 16.384 píxeles por lado. La selección total se limita a 512 MiB.
- Lienzos de salida de hasta 24 millones de píxeles y 32.768 por lado. Los paquetes generados se limitan a 256 MiB.
- Atlas de hasta 256 imágenes, celdas de 1 a 4096 píxeles y padding de 0 a 256. Las cuadrículas se limitan a 256 fotogramas.
- La imagen de revisión está limitada a 1200 × 1600 píxeles y 8 MiB. Los archivos generados conservan la resolución configurada.

Se usa la orientación de píxeles almacenada y el primer fotograma. Si una fotografía depende de una rotación EXIF, prepara una copia orientada antes de producir los assets. Los originales no se sobrescriben. Los manifiestos incluyen nombres y geometría de las imágenes elegidas; no incluyen rutas locales completas, cookies, sesiones ni URLs de origen.

Los archivos de entrada se validan como archivos locales regulares y se fuerzan los demultiplexores de PNG/JPEG. No se aceptan filtros, URLs de entrada, argumentos FFmpeg ni scripts arbitrarios. Los ZIP se escriben con el mismo helper nativo de exportación portable, mediante archivos generados y nombres relativos controlados.

## Verificación

```powershell
$env:NABNAB_TEST_ENGINES = (Resolve-Path .cache/ffmpeg-build/bundle).Path
cargo test --manifest-path src-tauri/Cargo.toml image_assets:: -- --include-ignored --test-threads=1
```

La prueba nativa genera imágenes sintéticas con FFmpeg, produce los cuatro resultados y vuelve a decodificar los PNG. Comprueba alfa y composición de fondo, entradas PNG del ICO, orden y coordenadas del atlas y de la cuadrícula, solapamiento conocido, píxeles de la unión, integridad de originales y cancelación. El test necesita motores preparados deliberadamente; no los instala por sí mismo.

`tests/image-assets.browser.mjs` comprueba los formularios con respuestas IPC de prueba: orden, tipos de opciones, rechazo de una previsualización antigua, cancelación, escape de nombres y ausencia de desbordamiento horizontal. Esa prueba de interfaz no sustituye la ejecución nativa ni acredita los datos de su PNG de prueba como una transformación real.

El recorrido integrado con la aplicación de escritorio, cola y biblioteca reales también pasó con archivos sintéticos: [evidencia nativa de la entrega C](verification/asset-workflows-native.json). Las capturas nativas están en [la guía integrada](ONLINE-LOCAL.md).
