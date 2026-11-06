# Podcasts RSS

En Biblioteca → Fuentes, añade una fuente y elige Podcast RSS. Pega el enlace del feed, revisa el historial y selecciona episodios para añadirlos a la cola común. Las comprobaciones, límites, pausas, revisión inicial y permisos de descarga automática reutilizan el seguimiento existente. La importación OPML solo añade suscripciones con comprobación manual; no descarga ni explora sus episodios automáticamente.

Los feeds se leen con un límite de 4 MiB y 10000 episodios. Cada página de la interfaz respeta el límite de exploración de la fuente. El servidor puede haber retirado episodios antiguos; una exploración completa solo representa el contenido que entrega ese feed. Se admiten RSS UTF-8 con enclosures de audio o vídeo. No se anuncia soporte Atom ni feeds con DTD. Título, autor, duración, tamaño y fecha se conservan cuando existen. Un valor desconocido sigue siendo desconocido.

La identidad de episodio usa el GUID cuando es no vacío y único en el feed. Sin un GUID fiable, usa el origen y la ruta del enclosure, excluyendo consulta y fragmento. Así, renovar una firma de descarga no duplica el episodio. Esta alternativa no puede distinguir dos episodios distintos alojados en una misma ruta y diferenciados únicamente por parámetros. Las identidades incluyen el feed para evitar mezclar publicaciones de suscripciones diferentes.

Las URL operativas del feed y los enclosures se guardan con el almacén protegido existente. La interfaz y el estado ordinario reciben el origen y una referencia opaca, no los tokens. No se transfieren cookies ni cabeceras de autorización del feed a los enclosures. Cada petición RSS fija la resolución DNS pública y vuelve a validar cada redirección. Los adjuntos pasan por la protección de red del descargador HTTP existente.

Las suscripciones son privadas por defecto y se excluyen de OPML. Al añadir una fuente puede declararse explícitamente pública para permitir exportar su enlace. Los feeds importados desde OPML vuelven a considerarse privados. La exportación informa de cuántas suscripciones omite. Eliminar el seguimiento conserva archivos y trabajos completados.

Desde una fuente Podcast, Exportar M3U genera una lista UTF-8 con los archivos locales descargados, en el orden observado en la fuente. No incluye enlaces remotos ni archivos ausentes. No sobrescribe una lista existente.

## Audio de plataformas

SoundCloud y Bandcamp utilizan los extractores de yt-dlp instalados, cuando el resolvedor de plataformas los habilita. El [extractor oficial de SoundCloud](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/soundcloud.py) distingue pistas y listas; la disponibilidad depende del acceso y los formatos que entregue el servicio. El [extractor oficial de Bandcamp](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/bandcamp.py) distingue pistas y álbumes. Un stream o una muestra no acredita acceso al archivo de compra. No se sustituyen canciones por vídeos de otros servicios.

## Verificación

Los fixtures Rust comprueban orden, metadatos, identidad GUID, firmas cambiantes, GUID repetidos, exclusión de destinos reservados, rechazo de DTD y lectura/exportación OPML. Las credenciales privadas y la reproducción de un episodio externo requieren una comprobación autorizada en vivo. No se descargó contenido personal durante la implementación.
