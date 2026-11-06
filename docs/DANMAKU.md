# Danmaku de Bilibili

La operación Exportar danmaku crea una pista XML independiente desde una página normal de vídeo Bilibili, identificada por `BV…` o `av…`. En vídeos de varias partes, `?p=2` selecciona la segunda parte. No descarga el vídeo, no incrusta comentarios en él y no ofrece ASS, SRT o WebP como si fueran formatos danmaku implementados.

El módulo consulta `https://api.bilibili.com/x/web-interface/view` con el identificador de vídeo, valida que la respuesta pertenece a ese vídeo y obtiene el CID de la parte seleccionada. Después recupera únicamente `https://comment.bilibili.com/<cid>.xml`. Es la ruta de pista danmaku que también declara el [extractor Bilibili de yt-dlp](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/bilibili.py). No se copió código del extractor ni se utilizan intermediarios de extracción.

Solo se exporta el XML accesible que el servidor devuelve en ese momento. Puede ser un conjunto retenido o limitado; no representa todo el historial de comentarios del vídeo. Los vídeos de temporadas o cursos, directos y enlaces cortos requieren otros recorridos y no se anuncian como cubiertos por esta operación. Si el API exige autorización, una firma adicional o rechaza el acceso, el trabajo informa del bloqueo. No reutiliza cookies, inicia sesión ni promete contenidos o calidades premium.

Las conexiones son HTTPS a los dos hosts oficiales indicados, con DNS público comprobado y fijado, redirecciones manuales al mismo host y límites de tiempo y tamaño. El límite de metadatos es 2 MiB y el de XML 32 MiB, hasta 200 000 comentarios y 16 KiB de texto por comentario. No se recuperan páginas de historial adicionales automáticamente.

El XML se valida y normaliza conservando orden, parámetros temporales y texto. Rechaza DTD, entidades externas, instrucciones de procesamiento y contenido activo o anidado dentro de un comentario. Los datos de CDATA y caracteres especiales se guardan como texto escapado. Un documento vacío, una respuesta HTML o un error del proveedor no produce una exportación marcada como correcta.

El archivo nuevo pasa por la cola y biblioteca comunes. No sobrescribe originales ni ejecuta scripts de los comentarios. No necesita otro componente aparte de las dependencias ya presentes en nabnab.

Los fixtures cubren identificación de vídeo y parte, correspondencia entre vídeo y CID, orden y tiempos del XML, CDATA y entidades, rechazo de documentos activos y ausencia de éxito vacío. No se ha probado la exportación contra un vídeo o cuenta Bilibili en vivo durante esta implementación.
