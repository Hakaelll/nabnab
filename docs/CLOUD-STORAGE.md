# Archivos de Drive, OneDrive y Dropbox

Los conectores leen los archivos accesibles a una cuenta autorizada. Las selecciones pasan por la cola de nabnab y producen archivos nuevos en Biblioteca. No suben documentos, sincronizan carpetas, borran elementos remotos ni ejecutan lo descargado.

## Configurar una aplicación OAuth legítima

No se incluyen identificadores ni credenciales de una aplicación de terceros. Cada proveedor necesita una aplicación registrada por quien distribuye o prueba esta compilación. Configúrala desde la conexión correspondiente y después pulsa Conectar. La autorización se abre en el navegador del sistema. nabnab nunca solicita la contraseña de la cuenta.

El retorno local es `http://127.0.0.1:53687/callback`. El puerto solo escucha durante la autorización, durante un máximo de tres minutos. No es un servicio del sistema. Todas las conexiones usan Authorization Code con PKCE S256, estado aleatorio de 256 bits y un código de un solo recorrido. Cancelar cierra el listener. El puerto ocupado se informa como error y no se sustituye por un destino remoto.

### Google Drive

1. Crea un proyecto propio en Google Cloud, activa Google Drive API y configura la pantalla de consentimiento y sus usuarios de prueba.
2. Registra un cliente de tipo **Desktop app**. Introduce su `client_id` en nabnab. Si el endpoint exige el `client_secret` generado para ese cliente de escritorio, el campo opcional permite usarlo como metadato de cliente público. No lo trates como un secreto que una aplicación distribuida pueda ocultar.
3. Autoriza `https://www.googleapis.com/auth/drive.readonly`. El navegador de carpetas necesita leer la cuenta: este alcance es más amplio que los archivos finalmente descargados y Google puede exigir verificación para distribución pública. No se pide escritura. El alcance `drive.file` por sí solo no da acceso al navegador completo y aquí no se simula un Google Picker.

Los documentos nativos muestran solamente los formatos reconocidos que la API ofrece en `exportLinks` para ese archivo. Un documento de texto puede ofrecer DOCX; una presentación PPTX; una hoja XLSX. No se intercambian esos formatos entre tipos. Se comprueba `capabilities.canDownload` y se resuelve de nuevo el archivo al ejecutar el trabajo. Los binarios descargan el original. La exportación nativa la genera Google, no el motor offline de nabnab; está sujeta a los límites de `files.export`, incluidos sus 10 MB de salida y los permisos reales.

Se admiten enlaces oficiales de archivo, documento y carpeta accesibles a la cuenta. Los enlaces que exigen `resourcekey` requieren seleccionar el elemento por el navegador de la cuenta y no se transforman por ensayo de parámetros. Los accesos directos se muestran y no se siguen automáticamente.

Referencia: [OAuth para escritorio](https://developers.google.com/identity/protocols/oauth2/native-app), [descargas y exportaciones](https://developers.google.com/workspace/drive/api/guides/manage-downloads), [files.list](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list).

### OneDrive

1. Registra una aplicación propia en Microsoft Entra con los tipos de cuenta que debas admitir, incluidas cuentas personales si corresponde.
2. Configura la plataforma **Mobile and desktop applications**, el retorno de bucle local indicado y el flujo de cliente público con navegador del sistema. No registres este recorrido como una SPA ni crees un secreto de cliente para distribuirlo.
3. Introduce el Application client ID. La autorización solicita `Files.Read offline_access`, sin escritura. Las políticas del tenant pueden impedir el acceso o exigir consentimiento administrativo.

El navegador usa Microsoft Graph v1.0. Las identidades conservan la unidad y el elemento; los elementos compartidos remotos conservan la unidad de destino. Los enlaces `1drv.ms`, `onedrive.live.com` y de SharePoint se resuelven mediante la API `/shares`, sin asumir que cambiar la URL basta. La ejecución consulta metadatos y solicita `/content`. Una redirección entrega una URL temporal que se descarga sin el token de Graph. Esa URL no forma parte del trabajo, del historial ni de los resultados serializados.

Referencia: [flujo de código con PKCE](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow), [driveItem content](https://learn.microsoft.com/en-us/graph/api/driveitem-get-content?view=graph-rest-1.0).

### Dropbox

1. Crea una aplicación Scoped API en Dropbox App Console. El acceso **App folder** limita la selección a esa carpeta. Usa **Full Dropbox** solamente si necesitas examinar archivos preexistentes fuera de ella.
2. Registra exactamente el retorno de bucle local y habilita `files.metadata.read`, `files.content.read`, `sharing.read` y `account_info.read`. No se pide escritura.
3. Introduce la app key como client ID. PKCE funciona sin app secret; se solicita acceso offline para renovar el token.

Las carpetas se recorren con `list_folder` y `list_folder/continue`; los archivos de la cuenta se descargan con `files/download`. Un enlace compartido de archivo se consulta con `get_shared_link_metadata` y se descarga con `get_shared_link_file`. El vínculo se guarda bajo DPAPI y la cola solo conserva una referencia opaca ligada a la cuenta, nunca la URL del enlace. Los enlaces de carpetas deben ser accesibles desde el navegador de la cuenta autorizada. No se admite introducir contraseñas de enlaces ni resolver documentos no descargables como si fueran binarios.

Referencia: [OAuth y PKCE](https://docs.dropboxapi.com/dropbox-api/docs/oauth), [acceso a archivos](https://docs.dropboxapi.com/dropbox-api/docs/file-access), [enlaces compartidos](https://docs.dropboxapi.com/dropbox-api/docs/sharing).

## Seleccionar y descargar

El navegador carga hasta 100 entradas por página y marca si quedan páginas o si Drive declara una búsqueda incompleta. La selección de una carpeta debe revisarse antes de encolarla. Su previsualización explora como máximo 500 archivos, 20 páginas de API y 12 niveles. Detecta carpetas repetidas y omite accesos directos. Al alcanzar un límite, el resultado queda marcado como parcial: no representa una copia completa de la carpeta.

Una cuenta puede estar conectada por proveedor. Los trabajos conservan el proveedor, la identidad de cuenta, la identidad del archivo, el formato de exportación y la versión seleccionada. Al ejecutar se consultan de nuevo permisos y versión. Si el origen cambió, el trabajo pide refrescar y seleccionar la nueva versión. Cambiar de cuenta no autoriza trabajos de la cuenta anterior. Los nombres recibidos se adaptan a un nombre local seguro y la publicación normal de la cola resuelve colisiones sin sobrescribir.

Los tokens y la configuración OAuth se guardan en el almacén DPAPI existente. Las cabeceras bearer se marcan sensibles. El cliente autenticado no sigue redirecciones y solo acepta los endpoints HTTPS fijos de cada API. Los fallos no repiten respuestas ni URLs firmadas del proveedor. Desconectar elimina los tokens locales; para retirar también el consentimiento en el proveedor, utiliza su panel de aplicaciones autorizadas. Los archivos ya descargados se conservan. Los enlaces Dropbox protegidos conservados sirven solo con la cuenta autorizada y pueden eliminarse junto con los datos locales de la aplicación.

Las transferencias tienen un límite de 128 GiB, escriben en staging, admiten cancelación, rechazan respuestas truncadas y borran su archivo parcial ante error. Las peticiones de metadatos tienen un límite de dos minutos y las transferencias autenticadas de 24 horas, con un máximo de 30 segundos sin recibir datos. Las descargas redirigidas de OneDrive reutilizan los límites del descargador HTTP existente. Si el proveedor exige un flujo que este conector no admite, se informa el requisito en vez de enviar el token a un sitio diferente.

## Comprobación

`cargo test --manifest-path src-tauri/Cargo.toml cloud_storage::tests` comprueba el vector PKCE de RFC 7636, el estado y los parámetros duplicados del retorno OAuth, las restricciones de endpoints, las exportaciones ofrecidas por MIME, la exclusión de accesos directos y archivos no descargables, las rutas compartidas oficiales y los nombres locales seguros. Los tests de transferencia comprueban contenido, cancelación y salidas parciales sin cuentas personales.

No se han introducido client IDs de desarrollo ni se ha abierto una sesión personal para esta implementación. Por tanto, las pruebas locales no acreditan una autorización real, una exportación de Google Docs o el acceso vivo a ninguna cuenta. Esos recorridos requieren la aplicación OAuth configurada y el consentimiento del titular.
