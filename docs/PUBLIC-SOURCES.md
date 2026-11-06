# Archivos de fuentes públicas

La categoría Archivos y documentos de Plataformas permite abrir repositorios o releases de GitHub, ítems y colecciones de Internet Archive y páginas File: o Category: de Wikimedia Commons. El análisis muestra elementos para elegir. No descarga todos los archivos ni ejecuta software publicado.

## Selección y límites

- GitHub presenta versiones publicadas, distinguiendo prereleases. Abre una versión para seleccionar sus assets. Los archivos fuente generados automáticamente por GitHub no se confunden con los assets publicados. La lista tiene páginas de 50 elementos, hasta 100 páginas. Sistema y arquitectura se deducen únicamente de nombres explícitos; la selección de archivo siempre es manual.
- Internet Archive presenta originales y derivados por separado, con título, descripción, procedencia y derechos disponibles. Tiene páginas de 50 registros y hasta 100 páginas. Una colección muestra sus miembros directos para abrirlos, sin recorrer colecciones anidadas ni encolar una colección completa. Los ítems restringidos, oscuros, sin descarga o que requieren préstamo se rechazan. Los archivos privados, DRM y rutas de traversal quedan excluidos.
- Commons ofrece el original y una variante de hasta 1280 píxeles cuando la API la devuelve. Una categoría carga 10 archivos por página, hasta 200 archivos directos. No recorre subcategorías. Los metadatos enriquecidos se piden por bloques de cinco archivos. Las licencias, autorías y créditos se conservan por archivo; la ausencia de datos no se convierte en una declaración de dominio público. El HTML de los metadatos se reduce a texto plano.

Las páginas incompletas muestran continuación o el límite alcanzado. Cancelar el análisis interrumpe solicitudes y lectura de metadatos. Las respuestas están limitadas a 8 MiB; las APIs oficiales no siguen redirecciones y sus conexiones fijan las direcciones DNS públicas comprobadas. No se envían cookies ni tokens. Las descargas utilizan el transporte HTTP y la cola comunes.

## Seguir versiones de GitHub

En Fuentes, elige GitHub Releases y pega el repositorio. La primera exploración registra el historial o lo presenta para revisión, según la selección. Las comprobaciones son manuales por defecto. Un intervalo solo se aplica si lo eliges expresamente, funciona con nabnab abierto y puede pausarse o eliminarse como el resto de fuentes.

Las nuevas versiones aparecen con su distinción estable o previa. Pulsa Elegir archivos para abrir el selector de esa release y añadir únicamente los assets necesarios a la cola. El seguimiento no descarga todos los paquetes de sistemas y arquitecturas distintos ni instala software. La descarga automática de releases está deshabilitada también en el servicio nativo. Este flujo reutiliza Fuentes y el selector público, sin otro historial de descargas.

La cola conserva el origen y la identidad del archivo. Antes de descargar vuelve a consultar sus metadatos y comprueba que sigue disponible para ese origen. Los assets de GitHub quedan vinculados al repositorio, versión e identificador del asset. Cuando GitHub publica un digest SHA-256 en la API, se conserva para comprobar integridad. Un checksum no prueba la identidad del publicador ni sustituye una firma de confianza.

Los resultados de Commons incluyen una ficha de atribución mediante la integración de cola. Los originales conservan sus archivos y tipos. No se altera el contenido para incrustar derechos ni se ejecuta ningún instalador descargado.

## Verificación

Las pruebas `public_sources` cubren análisis de URLs, identidad de releases, distinción de prereleases, validación de SHA-256, restricciones de Internet Archive, paginación, variantes de Commons, conversión de metadatos a texto y cancelación previa a la red. Son pruebas de metadatos de ejemplo, no pruebas de acceso real a cada servicio.

El recorrido `live_public_sources` está ignorado por defecto y consulta las tres APIs públicas. Se ejecuta explícitamente con:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml public_sources::tests::live_public_sources -- --ignored --nocapture
```

Ese recorrido prueba listado y resolución de identidades. No certifica una descarga completa, permisos de contenido restringido, todas las colecciones o todas las variantes. Las APIs pueden imponer límites o retirar archivos.

El recorrido separado `live_public_downloads` descarga muestras menores de 512 KiB, vuelve a resolver sus identidades, utiliza el transporte HTTP común e indexa el archivo real en una biblioteca temporal de prueba. El 6 de noviembre de 2026 ambos recorridos pasaron con acceso de red explícito:

- GitHub CLI `gh_2.103.3_checksums.txt`: 1971 bytes, indexado como documento y con digest SHA-256 comprobado cuando la API lo proporcionó.
- Wikimedia Commons `Example.jpg`: 9022 bytes, indexado como imagen.
- Internet Archive, ítem `BigBuckBunny_328`, `__ia_thumb.jpg`: 9692 bytes, indexado como imagen.

Las muestras se eliminaron de sus carpetas temporales propias. La prueba verifica selección, resolución, transferencia e indexación, no la publicación de un trabajo desde la interfaz de escritorio. Los metadatos reales de Commons incluían parámetros `utm_source`, `utm_campaign` y `utm_content`; el adaptador los elimina, conservando el rechazo de parámetros de acceso o URLs temporales. Una repetición anterior recibió un HTTP 504 de GitHub y la comprobación posterior pasó, sin tratar ese error temporal como un resultado completo.

```powershell
cargo test --manifest-path src-tauri/Cargo.toml public_sources::tests::live_public -- --ignored --nocapture --test-threads=1
```

## Referencias

Implementación propia sobre APIs públicas, sin copiar código de Omniget ni añadir un motor de descarga independiente. Referencias oficiales comprobadas el 6 de noviembre de 2026:

- [GitHub Releases](https://docs.github.com/en/rest/releases/releases) y [Release Assets](https://docs.github.com/en/rest/releases/assets).
- [Internet Archive: lectura de metadatos](https://archive.org/developers/md-read.html) y [búsqueda por metadatos y colecciones](https://archivesupport.zendesk.com/hc/en-us/articles/360018359991-Search-A-Basic-Guide).
- [MediaWiki Imageinfo](https://www.mediawiki.org/wiki/API:Imageinfo), [Categorymembers](https://www.mediawiki.org/wiki/API:Categorymembers) y [Commons API](https://commons.wikimedia.org/wiki/Commons:API/MediaWiki).
