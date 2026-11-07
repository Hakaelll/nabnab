# Servidores personales

Abre **Plataformas → Archivos y documentos → Servidores personales**. Las conexiones también se pueden gestionar en **Ajustes → Conexiones**.

- **Nextcloud / WebDAV.** Indica la URL HTTPS de la carpeta autorizada, el usuario y una contraseña de aplicación. En Nextcloud, copia la URL WebDAV desde los ajustes de Archivos. Windows debe confiar en el certificado HTTPS de la instancia. Las credenciales se guardan con DPAPI y se usan solo en esa conexión.
- **SFTP.** Indica host, puerto, usuario y raíz absoluta. Elige una clave privada local que funcione sin interacción con el cliente OpenSSH de Windows. Compara la huella SHA-256 mostrada con la que te haya dado el administrador antes de guardarla. Un cambio de clave detiene la conexión. Verificar y sustituir la identidad del servidor exige retirar primero la conexión anterior.
- **Carpeta de red / NAS.** Elige una carpeta con el selector nativo. Admite carpetas montadas y rutas UNC ya accesibles con la sesión de Windows. Una unidad de red se guarda como su ruta UNC para conservar la identidad de la ubicación aunque cambie la letra asignada.

Abre la conexión, navega por carpetas y marca archivos. **Indexar referencia** guarda los metadatos en **Biblioteca → Referencias**. Una referencia depende del servidor y no contiene una copia. **Copiar a mi biblioteca** muestra el destino y envía la selección a Descargas. Las referencias guardadas también ofrecen **Obtener copia local**.

Las copias verifican los metadatos seleccionados y comprueban el recurso al finalizar. Los reintentos empiezan con una copia nueva. Para WebDAV se exige un ETag fuerte o un tamaño y una fecha de modificación comprobables. SFTP v3 comprueba tamaño y fecha de modificación en el archivo abierto y en la ruta. La precisión de esta última depende del servidor. Un archivo con metadatos insuficientes se puede indexar como referencia.

La exploración consulta una carpeta a la vez y tiene límites de profundidad y tamaño. Las conexiones se limitan a lectura. Desconectar conserva las referencias y copias existentes. El índice de referencias se consulta en la interfaz local.

[WebDAV de Nextcloud](https://docs.nextcloud.com/server/stable/developer_manual/client_apis/WebDAV/basic.html) · [OpenSSH](https://www.openssh.com/manual.html) · [Rutas UNC en Windows](https://learn.microsoft.com/dotnet/standard/io/file-path-formats)

Las pruebas nativas comprobaron el contenido de una copia desde una carpeta de prueba de Windows, el rechazo de una versión que cambió y las validaciones de WebDAV y SFTP. La prueba de interfaz utilizó conexiones y huellas simuladas. Queda por comprobar una transferencia completa con una instancia Nextcloud o un servidor SFTP real.

![Selección de un archivo remoto y destino de una copia, con datos de prueba](personal-server-selection.png)
