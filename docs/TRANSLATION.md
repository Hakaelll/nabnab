# Traducir subtítulos

Abre **Herramientas → Subtítulos → Traducir subtítulos** o elige **Traducir pista** en el inspector de Biblioteca. Puedes usar una pista ya asociada a un archivo o importar un SRT o VTT. En **Ajustes → Componentes**, instala la dirección que necesites. Español → Inglés e Inglés → Español se ejecutan directamente.

1. Selecciona la pista y revisa los primeros segmentos antes de traducir.
2. Abre el borrador cuando termine la tarea. El original y la traducción se muestran en columnas, con el identificador y el intervalo de cada segmento. Edita el texto y guarda los cambios. La revisión sigue marcada como traducción automática.
3. Exporta SRT o VTT. Activa **Original y traducción** para incluir las dos versiones en cada segmento. VTT conserva los identificadores de texto. SRT usa identificadores numéricos. Cada exportación es un archivo nuevo vinculado a su entrada en Biblioteca.

El motor agrupa segmentos cercanos cuando puede recuperar cada traducción en orden. Si un bloque pierde la correspondencia, traduce sus segmentos por separado. Se conservan los tiempos y las etiquetas simples de negrita, cursiva y subrayado cuando las mantiene el motor. La herramienta avisa si un segmento supera 42 caracteres por línea o 20 por segundo. VTT conserva tiempos a milisegundos. Los estilos de posición, regiones y CSS quedan en el archivo original.

## Componentes

Bergamot 0.4.9 requiere Windows x64 y usa el Deno de nabnab. Tanto el motor como los pesos se distribuyen bajo MPL-2.0. La primera instalación descarga aproximadamente entre 15,8 y 16,6 MiB, según la dirección. El runtime y el vocabulario compartidos ocupan 5,8 MiB. Una segunda dirección descarga únicamente sus propios pesos y tabla léxica. Los archivos descomprimidos y el paquete del motor se verifican con SHA-256. [Motor y procedencia](https://github.com/browsermt/bergamot-translator/tree/main/wasm/module) · [Modelos de Firefox](https://github.com/mozilla/firefox-translations-models)

La inferencia ejecuta Deno con acceso a las carpetas de entrada y de modelo estrictamente necesarias y con red, escritura, ejecución externa y FFI denegados. Los temporales y la caché de ese proceso se retiran al terminar. Desactivar una dirección conserva sus archivos. Eliminarla libera su espacio y elimina el runtime compartido si era la última. Los borradores y las exportaciones se conservan. Puedes retirar un borrador desde Herramientas para liberar su espacio.

## Verificación

El recorrido nativo de Windows instaló las dos direcciones desde Ajustes, con un único runtime y vocabulario. Tradujo un VTT local, conservó cuatro identificadores y sus tiempos, y mantuvo la cursiva. Se guardó una corrección, se reabrió el borrador y se verificaron el VTT bilingüe y el SRT traducido. El original quedó idéntico y los directorios temporales quedaron vacíos. La comparación se comprobó a 1180 y 900 px. [Comparación y aviso de longitud](translation-preview.png)
