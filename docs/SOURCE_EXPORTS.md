# Exportar información de fuentes

Las exportaciones crean trabajos reales en la cola y guardan archivos en la biblioteca. No descargan vídeos. Conservan las sesiones existentes y las URL operativas protegidas.

YouTube permite guardar por separado la miniatura accesible en su formato original, descripción TXT, metadatos JSON, capítulos JSON o Markdown y metadatos de una lista JSON. La miniatura tiene un límite de 2 MiB. Los metadatos exportados usan una lista de campos públicos; se excluyen formatos de descarga, URL firmadas, cookies y cabeceras internas del motor. Las listas incluyen hasta 200 posiciones y registran el alcance, el total conocido y si el resultado es parcial. No se inventan capítulos ni se crean descripciones vacías.

Reddit permite exportar una publicación y los comentarios que devuelva una petición acotada de su API JSON. Se usa la sesión de Reddit existente cuando está disponible, solo en el origen oficial. El límite es 200 comentarios y una profundidad acotada. El archivo distingue comentarios recuperados, total declarado y comentarios adicionales indicados por Reddit que no se han cargado. No afirma que sea un archivo íntegro. Un bloqueo de acceso se muestra como tal.

X/Twitter permite exportar los textos que gallery-dl devuelve para una conversación accesible, incluyendo publicaciones sin adjuntos. Se requiere el componente Galerías y puede requerirse una sesión autorizada de X. Se consultan X y su API directamente, sin mirrors. El límite es 200 publicaciones. El archivo conserva autores, enlaces, fechas disponibles y relaciones de respuesta. La conversación recuperada puede incluir respuestas de otras personas y omitir publicaciones restringidas, borradas o no devueltas. Nunca se marca como hilo certificado completo. Los errores parciales del motor se conservan como avisos redactados.

Reddit y X admiten JSON, Markdown y HTML estático. El HTML no contiene scripts ni recursos remotos incrustados y tiene una política de contenido restrictiva. El Markdown escapa el texto externo para evitar que imágenes o HTML de la publicación provoquen cargas silenciosas. Los enlaces al origen requieren una acción del lector.

Las opciones se contrastaron con las interfaces oficiales de [yt-dlp](https://github.com/yt-dlp/yt-dlp), [Reddit](https://www.reddit.com/dev/api/) y [gallery-dl 1.31.10](https://github.com/mikf/gallery-dl/blob/v1.31.10/gallery_dl/extractor/twitter.py), incluyendo `conversations=accessible`, `text-tweets` y el límite por publicaciones `--post-range`. No se copió código de los extractores.

Los fixtures comprueban exclusión de datos operativos sensibles, alcance de listas, comentarios ausentes, conversaciones con texto sin medios, errores de extracción y HTML seguro. Queda pendiente verificar cada recorrido en vivo con contenido y cuentas autorizados. No se descargó contenido personal durante la implementación.
