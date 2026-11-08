# Investigación y audiolibros

En **Plataformas → Archivos y documentos**, pega un enlace, un identificador o un DOI. Selecciona los archivos, revisa el tamaño y el destino, y usa **Añadir a Descargas**. También puedes pegar enlaces de estos orígenes en Inicio. Las búsquedas de arXiv y LibriVox se pueden guardar en **Biblioteca → Fuentes** para revisarlas manualmente. Dentro de una ficha, **Volver a resultados** recupera las páginas ya cargadas y devuelve el foco a la fila que abriste.

## arXiv

- Busca por título, autor o categoría. Puedes abrir un identificador como `arXiv:2401.00001` o un enlace de artículo.
- Cada PDF tiene su propia fila con número de versión, nombre de archivo y fecha si el origen la indica. Puedes desplegar el resumen en la misma ficha. La referencia guardada identifica la versión exacta y conserva autores, fecha y licencia cuando los informa el origen.
- La búsqueda consulta hasta 20 resultados por página y 200 por búsqueda. La API Atom se consulta de una en una, con tres segundos entre peticiones y una caché acotada en memoria.

[Manual de la API](https://info.arxiv.org/help/api/user-manual.html) · [Condiciones de la API](https://info.arxiv.org/help/api/tou.html) · [Disponibilidad por versión](https://info.arxiv.org/help/versions.html)

## Zenodo

- Abre un enlace de registro, un ID numérico o un DOI. La ficha distingue el registro concreto del conjunto al que pertenece. Cada archivo queda ligado al registro concreto, aunque se haya abierto mediante un DOI de conjunto.
- **Historial de versiones** enumera los registros públicos de la familia. Abre una versión para elegir sus archivos o usa **Seguir versiones** para revisar las novedades manualmente desde Biblioteca.
- Revisa tipo, versión, tamaño y checksum de los archivos. La selección muestra el tamaño total conocido antes de enviar nada a la cola. Se verifica el checksum publicado MD5 o SHA-256 cuando esté disponible.
- Los registros con archivos restringidos, embargados o sin acceso público declarado muestran metadatos sin permitir la descarga. La cola guarda archivos sin ejecutarlos.

[API de registros y límites](https://developers.zenodo.org/#records) · [Versiones de registros](https://help.zenodo.org/docs/deposit/manage-versions/)

## LibriVox

- Busca por el comienzo del título y, opcionalmente, por idioma tal como aparece en el catálogo. Abre la edición correspondiente. Los capítulos muestran número, lector e idioma cuando están disponibles.
- Si la edición tiene varios créditos de lectura, usa **Lectura** para ver los capítulos de uno de ellos. Marca capítulos individuales o selecciona los visibles. Cambiar de filtro conserva las marcas anteriores y el resumen muestra todo lo que se enviará a Descargas. Biblioteca identifica los archivos como audiolibro, conserva la edición y crea una colección para reunir sus capítulos. El filtro **Audiolibros** está en el selector de contenido de Biblioteca.
- El catálogo se consulta de 20 en 20. La aplicación lee el RSS y, para los archivos alojados en Internet Archive, reutiliza las comprobaciones de acceso y las descargas de ese adaptador. La ficha conserva la declaración de dominio público de LibriVox para Estados Unidos. Comprueba la situación en tu país.

[Catálogo oficial](https://librivox.org/api/info) · [Dominio público](https://librivox.org/pages/public-domain/)

## Cursos

El flujo de Rocketseat, Coursera, Domestika y Skillshare utiliza el sitio oficial para la sesión del alumno y la descarga permitida por ese sitio. En nabnab puedes importar la carpeta elegida y estudiar sus materiales con el orden y el progreso de Cursos. La [guía de cursos](COURSES.md) detalla qué materiales admite cada plataforma y el alcance de esta integración.

## Comprobaciones

Las capturas usan datos de prueba. La prueba de interfaz recorre la selección por versión, el retorno con teclado tras paginar o encontrar un error, y la selección de capítulos en ventanas de 1180 × 780 y 900 × 620. Las pruebas Rust cubren identidad por versión, lectura de metadatos, capítulos ordenados y seguimiento manual.

La prueba nativa descargó dos versiones distintas del mismo PDF de arXiv y comprobó que ambas se conservan. Descargó un CSV de Zenodo de 450 bytes y verificó el MD5 publicado. Descargó dos capítulos de LibriVox en orden inverso y comprobó el orden de lectura en Biblioteca. [Artículo de arXiv](https://arxiv.org/abs/2506.08399) · [Registro de Zenodo](https://zenodo.org/records/14634897) · [Edición de LibriVox](https://librivox.org/aesops-fables-volume-1-fables-1-25/)

Para repetir la prueba nativa, inicia Vite en el puerto 1420 y ejecuta `node tests/native-research.browser.mjs` con Playwright y el ejecutable de depuración de Windows disponibles. Los archivos y el perfil de prueba quedan en `artifacts/native-research`.

![Selección de PDF por versión de arXiv](arxiv-versions.png)

![Volumen y checksum antes de descargar un archivo de Zenodo](zenodo-selection.png)

![Capítulos seleccionados de una edición de LibriVox](librivox-chapters.png)
