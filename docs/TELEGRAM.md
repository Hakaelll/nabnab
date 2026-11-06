# Telegram para descargar medios

La integración usa el cliente JSON nativo de TDLib y la cuenta del usuario. No usa la Bot API, mirrors ni credenciales de otro proyecto. No envía mensajes, marca chats como leídos, se une a grupos, compra acceso ni crea cuentas.

## Componente y credenciales

El adaptador reconoce exactamente los esquemas de TDLib 1.8.0 y 1.8.69. Se necesita una compilación que exporte `td_json_client_create`, `td_json_client_send`, `td_json_client_receive`, `td_json_client_execute` y `td_json_client_destroy`. La interfaz está documentada en el [API JSON oficial](https://core.telegram.org/tdlib/docs/td__json__client_8h.html). Antes de enviar credenciales a TDLib, una solicitud local `getOption("version")` determina el esquema. La [versión 1.8.0](https://github.com/tdlib/td/blob/v1.8.0/td/generate/scheme/td_api.tl) recibe los parámetros anidados y la clave en el paso posterior; la [versión 1.8.69](https://github.com/tdlib/td/blob/master/td/generate/scheme/td_api.tl) utiliza parámetros planos y la clave incluida. Una versión distinta se rechaza con un mensaje explícito. No se certifica toda la rama 1.8 ni TDLib 2. Las diferencias de búsqueda y contenido efímero se tratan según esos esquemas.

No hay un binario TDLib verificado incluido ni un instalador automático para este módulo. El proyecto no descarga DLL de páginas visitadas ni de datos de Telegram. Para desarrollar en Windows x64, compila TDLib desde su [repositorio oficial](https://github.com/tdlib/td) siguiendo sus instrucciones y establece `NABNAB_TDLIB_DLL` con la ruta absoluta de `tdjson.dll` antes de iniciar una compilación de depuración de nabnab. Sus dependencias nativas deben estar junto a la DLL o en los directorios del sistema. La carga limita la búsqueda a esos directorios, sin buscar bibliotecas en el directorio de trabajo. Las compilaciones de distribución esperan un componente administrado `tdjson.dll`; falta preparar y verificar ese paquete antes de distribuirlo.

Obtén un `api_id` y un `api_hash` legítimos en [my.telegram.org](https://my.telegram.org). Quien distribuya la aplicación debe registrar su propia aplicación y cumplir las condiciones de Telegram. No se incluyen valores funcionales de muestra. Instalar TDLib no autoriza una cuenta.

TDLib utiliza la licencia Boost Software License 1.0. La nueva dependencia Rust `qrcodegen` 1.8.0 usa MIT y genera el QR localmente. No se envían tokens a un generador de códigos externo.

## Recorrido

1. Abre Telegram desde Plataformas. Elige una conexión local o configura una nueva, hasta un máximo de cinco.
2. Guarda las credenciales legítimas de aplicación y conecta. Autoriza con teléfono y código, contraseña de segundo factor cuando corresponda o QR mediante otro dispositivo que ya tenga sesión.
3. Si se muestra un QR, escanéalo desde Telegram → Ajustes → Dispositivos → Vincular dispositivo y pulsa Comprobar autorización en nabnab. Si la inicialización sigue en curso, utiliza la misma comprobación.
4. Carga hasta 100 chats de la lista principal, elige un chat y previsualiza una página de 25, 50 o 100 mensajes. También puedes abrir un enlace HTTPS `t.me` o `telegram.me` de un mensaje accesible. Esto no se une a un canal.
5. Filtra por fotos, vídeos, audio o documentos, texto, fecha y tamaño. Los filtros de rango se aplican a cada página inspeccionada. Una página sin resultados no representa todo el historial. El tamaño desconocido no coincide con un filtro por tamaño.
6. Selecciona hasta 200 adjuntos y añádelos a Descargas, con colección opcional. La cola conserva la identidad de cuenta, chat, mensaje y variante del adjunto. Cada archivo tiene su resultado y error individual. Cargar otra página es manual; no se descarga todo el historial al abrir un chat o enlace.

Se conserva la variante de mayor resolución disponible en una fotografía; no se cuentan sus miniaturas como adjuntos adicionales. Vídeos, animaciones, audio, notas de voz, videomensajes y documentos conservan su archivo accesible. Los álbumes de Telegram aparecen como los mensajes que los componen. No se implementa un cliente de mensajería completo, chats secretos ni recuperación de contenido eliminado o efímero.

## Sesión y permisos

Las credenciales de aplicación y la clave aleatoria de la base de TDLib se guardan mediante DPAPI para el usuario actual de Windows, en un directorio con permisos restringidos. La base de TDLib recibe esa clave de cifrado. Los códigos y contraseñas de autorización no se guardan en la configuración ni se registran. El flujo nativo desactiva la salida de logs de TDLib. Las copias temporales serializadas de la configuración se limpian al liberarse.

La caché de archivos de TDLib contiene los medios descargados en su formato original; no se presenta como un almacén de medios cifrado. Solo la sesión y su clave están protegidas. La copia final de cada adjunto pasa a la cola y biblioteca comunes. La aplicación no carga previsualizaciones de medios en la lista de Telegram por el mero hecho de navegar.

Antes de descargar, el módulo vuelve a consultar el chat y el mensaje. Exige `can_be_saved`, rechaza la protección del chat y contenido efímero, y comprueba que la identidad del archivo coincide. Repite la comprobación antes de copiar el resultado a la cola. El [API oficial de mensajes](https://core.telegram.org/tdlib/docs/classtd_1_1td__api_1_1message.html) define ese permiso; no se intenta eludirlo. Una restricción que cambie puede hacer que un trabajo falle aunque se hubiera seleccionado antes.

Desconectar cierra el cliente y conserva la sesión para volver a conectar. Eliminar una conexión requiere confirmación y una cuenta conectada. Si está autorizada, se solicita `logOut`; si aún espera teléfono, código o contraseña, se cierra el cliente sin solicitar un cierre de sesión inexistente. La clave y la etiqueta solo se eliminan después de recibir `authorizationStateClosed`. Si el cierre no se confirma, se conservan y se puede reintentar. Los archivos de Biblioteca se mantienen independientes. El [cierre de sesión de TDLib destruye sus datos locales](https://core.telegram.org/tdlib/docs/classtd_1_1td__api_1_1log_out.html), por lo que no se promete conservar su caché. Cualquier base residual queda cifrada sin su clave; una conexión nueva utiliza otro directorio y otra clave. Para revocar una sesión que no pueda conectarse, utiliza también los controles de dispositivos de Telegram.

Las descargas usan `downloadFile` y comprueban el estado de `getFile`, sin exponer la caché como rutas arbitrarias al formulario. La cancelación utiliza `cancelDownloadFile`. El archivo completo se valida bajo la caché de la cuenta, se comprueba su tamaño conocido y se copia a una salida nueva del trabajo, sin ejecutar su contenido. El caché permite reintentar sin empezar la transferencia desde cero cuando TDLib conserva el archivo.

## Estado de comprobación

Las pruebas locales cubren identidad y tamaño del adjunto, variante fotográfica, bloqueo de contenido protegido y efímero en ambos esquemas, selección de parámetros por versión, decisiones de cierre de sesión, autorización tipada, credenciales inválidas, Base64 de la clave y generación local de QR. Se ejecutan con `cargo test --manifest-path src-tauri/Cargo.toml telegram::tests`.

Falta comprobar con una DLL compatible y una cuenta autorizada la conexión real, teléfono/código, segundo factor, QR, lista de chats, paginación y descarga. No se ha cargado TDLib, iniciado sesión ni descargado contenido personal durante esta implementación. Los pasos de alta nueva, correo electrónico o suscripciones que TDLib pueda solicitar se identifican como no implementados, sin sustituirlos por un éxito.
