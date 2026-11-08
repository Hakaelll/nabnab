# Servidores personales

Abre **Plataformas → Archivos y documentos → Servidores personales**. Las conexiones también se pueden gestionar en **Ajustes → Conexiones**.

- **Nextcloud / WebDAV.** Indica la URL HTTPS de la carpeta autorizada, el usuario y una contraseña de aplicación. En Nextcloud, copia la URL WebDAV desde los ajustes de Archivos. Por defecto se usan los certificados de confianza de Windows. Para un servidor con una CA privada, abre **Certificado de confianza** y pega el certificado público PEM proporcionado por el administrador. Se usa solo para esa conexión y se verifica también el nombre del servidor. Si cambia el certificado de confianza, verifica el nuevo, desconecta la conexión anterior y vuelve a guardarla. Las credenciales se guardan con DPAPI.
- **SFTP.** Indica host, puerto, usuario y raíz absoluta. Elige una clave privada local que funcione sin interacción con el cliente OpenSSH de Windows. Compara la huella SHA-256 mostrada con la que te haya dado el administrador antes de guardarla. Un cambio de clave detiene la conexión. Verificar y sustituir la identidad del servidor exige retirar primero la conexión anterior.
- **Carpeta de red / NAS.** Elige una carpeta con el selector nativo. Admite carpetas montadas y rutas UNC ya accesibles con la sesión de Windows. Una unidad de red se guarda como su ruta UNC para conservar la identidad de la ubicación aunque cambie la letra asignada.

Abre la conexión, navega por carpetas y marca archivos. **Indexar referencia** guarda los metadatos en **Biblioteca → Referencias**. Una referencia depende del servidor y no contiene una copia. **Copiar a mi biblioteca** muestra el destino y envía la selección a Descargas. Las referencias guardadas también ofrecen **Obtener copia local**.

Desde una referencia, **Revisar en servidor** abre su carpeta y compara los metadatos guardados con los actuales. Si cambia el identificador de versión, el panel lo señala aunque tamaño y fecha coincidan. **Actualizar referencia** guarda los metadatos revisados. **Copiar a mi biblioteca** solicita una copia de la versión seleccionada. Si el archivo desaparece o la carpeta está inaccesible, la referencia sigue disponible para revisarla más tarde.

Las copias verifican los metadatos seleccionados y comprueban el recurso al finalizar. En WebDAV, si se interrumpe una copia con ETag fuerte y tamaño conocido, **Reanudar** solicita los bytes pendientes de esa versión. **Descartar parcial** elimina los bytes guardados. Si el servidor ignora el rango y devuelve el mismo archivo completo, la copia vuelve a empezar. Si el archivo cambió, nabnab descarta el parcial y pide elegir la versión nueva. Los demás reintentos empiezan con una copia nueva. [Rangos HTTP y combinación de fragmentos](https://www.rfc-editor.org/rfc/rfc9110.html#name-combining-parts)

Para copiar desde WebDAV se exige un ETag fuerte o un tamaño y una fecha de modificación comprobables. SFTP v3 comprueba tamaño y fecha de modificación en el archivo abierto y en la ruta. La precisión de esta última depende del servidor. Un archivo con metadatos insuficientes se puede indexar como referencia.

La exploración consulta una carpeta a la vez y tiene límites de profundidad y tamaño. Las conexiones se limitan a lectura. Desconectar conserva las referencias y copias existentes. El índice de referencias se consulta en la interfaz local.

[WebDAV de Nextcloud](https://docs.nextcloud.com/server/stable/developer_manual/client_apis/WebDAV/basic.html) · [Certificados de confianza en reqwest 0.13.5](https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html#method.tls_certs_only) · [OpenSSH](https://www.openssh.com/manual.html) · [Rutas UNC en Windows](https://learn.microsoft.com/dotnet/standard/io/file-path-formats)

![Configuración WebDAV y certificado opcional en una ventana compacta con datos de prueba](personal-server-webdav.png)

La prueba nativa en Windows guardó dos versiones de un archivo en una carpeta montada sintética y dos versiones a través de un servidor WebDAV HTTPS local. En ambos casos rechazó la referencia antigua aunque coincidían tamaño y fecha, y conservó las dos copias tras actualizar la referencia. En WebDAV también comprobó el certificado y el nombre antes de enviar credenciales, el rechazo de rutas fuera de la raíz y el bloqueo de redirecciones. Las huellas SFTP se comprobaron con fixtures. Queda por verificar una transferencia con una instancia Nextcloud, un servidor SFTP o un NAS en red.

Para repetir la prueba nativa, inicia Vite en el puerto 1420 y ejecuta `node tests/native-personal-servers.browser.mjs` con Playwright y el ejecutable de depuración de Windows disponibles. Define `NABNAB_TEST_OPENSSL` con la ruta a OpenSSL 3 para incluir el ensayo WebDAV y generar los certificados efímeros.

La prueba también cortó una copia HTTPS después de 96.000 bytes. Verificó el rango de reanudación, el resultado byte a byte, el reinicio ante una respuesta completa y el descarte de rangos o versiones incompatibles.

![Copia WebDAV interrumpida y acciones de recuperación en Windows, con datos de prueba](personal-server-resume.png)

![Selección de un archivo remoto y destino de una copia, con datos de prueba](personal-server-selection.png)

![Revisión de una versión distinta en la aplicación nativa de Windows](personal-server-review.png)
