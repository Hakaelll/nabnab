# Investigación y audiolibros

En **Plataformas → Archivos y documentos**, pega un enlace, un identificador o un DOI. Selecciona los archivos, revisa el tamaño y el destino, y usa **Añadir a Descargas**. También puedes pegar enlaces de estos orígenes en Inicio. Las búsquedas de arXiv y LibriVox se pueden guardar en **Biblioteca → Fuentes** para revisarlas manualmente.

## arXiv

- Busca por título, autor o categoría. Puedes abrir un identificador como `arXiv:2401.00001` o un enlace de artículo.
- La ficha contiene metadatos y, si existe, un resumen. El PDF se ofrece por versión. La referencia guardada identifica la versión exacta y conserva autores, fecha y licencia cuando los informa el origen.
- La búsqueda consulta hasta 20 resultados por página y 200 por búsqueda. La API Atom se consulta de una en una, con tres segundos entre peticiones y una caché acotada en memoria.

[Manual de la API](https://info.arxiv.org/help/api/user-manual.html) · [Condiciones de la API](https://info.arxiv.org/help/api/tou.html) · [Disponibilidad por versión](https://info.arxiv.org/help/versions.html)

## Zenodo

- Abre un enlace de registro, un ID numérico o un DOI. La ficha distingue el registro concreto del conjunto al que pertenece. Cada archivo queda ligado al registro concreto, aunque se haya abierto mediante un DOI de conjunto.
- Revisa tipo, versión, tamaño y checksum de los archivos. La selección muestra el tamaño total conocido antes de enviar nada a la cola. Se verifica el checksum publicado MD5 o SHA-256 cuando esté disponible.
- Los registros con archivos restringidos, embargados o sin acceso público declarado muestran metadatos sin permitir la descarga. La cola guarda archivos sin ejecutarlos.

[API de registros y límites](https://developers.zenodo.org/#records) · [Versiones de registros](https://help.zenodo.org/docs/deposit/manage-versions/)

## LibriVox

- Busca por el comienzo del título y, opcionalmente, por idioma tal como aparece en el catálogo. Abre la edición correspondiente. Los capítulos muestran número, lector e idioma cuando están disponibles.
- Marca capítulos individuales o selecciona los capítulos visibles. Biblioteca los identifica como audiolibro, conserva la edición y crea una colección para reunir sus capítulos. El filtro **Audiolibros** está en el selector de contenido de Biblioteca.
- El catálogo se consulta de 20 en 20. La aplicación lee el RSS y, para los archivos alojados en Internet Archive, reutiliza las comprobaciones de acceso y las descargas de ese adaptador. La ficha conserva la declaración de dominio público de LibriVox para Estados Unidos. Comprueba la situación en tu país.

[Catálogo oficial](https://librivox.org/api/info) · [Dominio público](https://librivox.org/pages/public-domain/)

## Cursos

El flujo de Rocketseat, Coursera, Domestika y Skillshare utiliza el sitio oficial para la sesión del alumno y la descarga permitida por ese sitio. En nabnab puedes importar la carpeta elegida y estudiar sus materiales con el orden y el progreso de Cursos. La [guía de cursos](COURSES.md) detalla qué materiales admite cada plataforma y el alcance de esta integración.

## Comprobaciones

Las capturas usan fixtures de interfaz. Las pruebas Rust cubren identidad por versión, lectura de metadatos, capítulos ordenados y seguimiento manual. Se consultó en vivo la ficha Atom de una versión de arXiv. En Zenodo se comprobó el formato del registro, la búsqueda por conjunto y la respuesta de un archivo público de 450 bytes. La consulta en vivo de LibriVox agotó el tiempo de espera.

![Selección de PDF por versión de arXiv](arxiv-versions.png)

![Volumen y checksum antes de descargar un archivo de Zenodo](zenodo-selection.png)

![Capítulos seleccionados de una edición de LibriVox](librivox-chapters.png)
