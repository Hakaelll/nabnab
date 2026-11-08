# Servidores personales

Abre **Plataformas → Archivos y documentos → Servidores personales**. Las conexiones también se pueden gestionar en **Ajustes → Conexiones**.

- **Nextcloud / WebDAV.** Indica la URL HTTPS de la carpeta autorizada, el usuario y una contraseña de aplicación. En Nextcloud, copia la URL WebDAV desde los ajustes de Archivos. Windows debe confiar en el certificado HTTPS de la instancia. Las credenciales se guardan con DPAPI y se usan solo en esa conexión.
- **SFTP.** Indica host, puerto, usuario y raíz absoluta. Elige una clave privada local que funcione sin interacción con el cliente OpenSSH de Windows. Compara la huella SHA-256 mostrada con la que te haya dado el administrador antes de guardarla. Un cambio de clave detiene la conexión. Verificar y sustituir la identidad del servidor exige retirar primero la conexión anterior.
- **Carpeta de red / NAS.** Elige una carpeta con el selector nativo. Admite carpetas montadas y rutas UNC ya accesibles con la sesión de Windows. Una unidad de red se guarda como su ruta UNC para conservar la identidad de la ubicación aunque cambie la letra asignada.

Abre la conexión, navega por carpetas y marca archivos. **Indexar referencia** guarda los metadatos en **Biblioteca → Referencias**. Una referencia depende del servidor y no contiene una copia. **Copiar a mi biblioteca** muestra el destino y envía la selección a Descargas. Las referencias guardadas también ofrecen **Obtener copia local**.

Desde una referencia, **Revisar en servidor** abre su carpeta y compara los metadatos guardados con los actuales. Si cambia el identificador de versión, el panel lo señala aunque tamaño y fecha coincidan. **Actualizar referencia** guarda los metadatos revisados. **Copiar a mi biblioteca** solicita una copia de la versión seleccionada. Si el archivo desaparece o la carpeta está inaccesible, la referencia sigue disponible para revisarla más tarde.

Las copias verifican los metadatos seleccionados y comprueban el recurso al finalizar. Los reintentos empiezan con una copia nueva. Para WebDAV se exige un ETag fuerte o un tamaño y una fecha de modificación comprobables. SFTP v3 comprueba tamaño y fecha de modificación en el archivo abierto y en la ruta. La precisión de esta última depende del servidor. Un archivo con metadatos insuficientes se puede indexar como referencia.

La exploración consulta una carpeta a la vez y tiene límites de profundidad y tamaño. Las conexiones se limitan a lectura. Desconectar conserva las referencias y copias existentes. El índice de referencias se consulta en la interfaz local.

[WebDAV de Nextcloud](https://docs.nextcloud.com/server/stable/developer_manual/client_apis/WebDAV/basic.html) · [OpenSSH](https://www.openssh.com/manual.html) · [Rutas UNC en Windows](https://learn.microsoft.com/dotnet/standard/io/file-path-formats)

La prueba nativa en Windows abrió una conexión sintética a una carpeta local, guardó una copia, simuló una desconexión y sustituyó el archivo por otro de igual tamaño y fecha. La cola rechazó la referencia antigua y, tras actualizarla desde la interfaz, guardó la segunda versión conservando la primera. Las conexiones y huellas de WebDAV y SFTP se comprobaron con fixtures. Queda por verificar una transferencia completa con una instancia Nextcloud, un servidor SFTP o un NAS en red.

![Selección de un archivo remoto y destino de una copia, con datos de prueba](personal-server-selection.png)

![Revisión de una versión distinta en la aplicación nativa de Windows](personal-server-review.png)
