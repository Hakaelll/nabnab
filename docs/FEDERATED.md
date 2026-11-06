# Instancias federadas

PeerTube y Mastodon se consultan por el origen del enlace, sin una lista fija de dominios. La exploración necesita aprobación explícita para contactar con esa instancia. Esa aprobación solo afecta a la consulta solicitada: no registra un origen de confianza, conecta una cuenta ni reutiliza cookies de otra plataforma.

PeerTube admite páginas `/w/id`, `/videos/watch/id`, `/video-playlists/watch/id` y `/w/p/id`. Las listas se cargan en páginas de hasta 50 vídeos. Se selecciona el archivo directo de mayor resolución que informa la API; los valores ausentes siguen siendo desconocidos. Se respetan las descargas desactivadas. Los manifiestos HLS/DASH y los directos activos no se presentan como archivos terminados.

Mastodon admite publicaciones `/@cuenta/id` y `/users/cuenta/statuses/id`. Enumera sus imágenes, vídeos, animaciones gifv y audio con identidades por adjunto. Una publicación debe ser pública o no listada y estar disponible sin autorización. Si la instancia exige OAuth, se informa del bloqueo; este recorrido no implementa aún una conexión OAuth.

La API usa peticiones GET sin autenticación, con DNS público fijado por conexión, límite de 8 MiB y redirecciones solo dentro del origen aprobado. La consulta completa tiene un límite de dos minutos. Las URL operativas de los adjuntos permanecen en el backend y se excluyen de la respuesta de la interfaz. La selección usa la cola y la biblioteca comunes; los hosts de medios y sus redirecciones se vuelven a comprobar al descargar. Los archivos pueden estar alojados en un CDN público del proveedor y no reciben cookies de la instancia.

Los fixtures comprueban reconocimiento de rutas, selección de archivo directo, metadatos ausentes, descargas desactivadas, identidades estables, adjuntos múltiples, paginación y exclusión de URL firmadas. Queda pendiente una prueba autorizada en vivo con instancias compatibles. Referencias: [API REST de PeerTube](https://docs.joinpeertube.org/api-rest-reference.html), [publicaciones de Mastodon](https://docs.joinmastodon.org/methods/statuses/) y [adjuntos de Mastodon](https://docs.joinmastodon.org/entities/MediaAttachment/).
