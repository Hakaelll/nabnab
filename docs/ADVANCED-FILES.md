# Archivos HTTP y HTTPS

El formulario Archivos permite inspeccionar un enlace directo, elegir de 1 a 16 conexiones y proporcionar un SHA-256 esperado. La descarga real usa el componente opcional aria2 y termina en la cola y biblioteca comunes. No instala componentes al abrirse, no ejecuta los archivos y no utiliza los permisos P2P de BitTorrent.

El motor solo utiliza más de una conexión si una petición real `Range: bytes=0-0` devuelve `206 Partial Content` con un `Content-Range` válido y tamaño conocido. El encabezado `Accept-Ranges` por sí solo no basta. Si no se confirma el rango, se usa una conexión. El tamaño y la disponibilidad de la fuente también pueden reducir las conexiones efectivas; no se promete una multiplicación de velocidad.

## Red y privacidad

aria2 no proporciona una opción documentada para fijar la resolución DNS y volver a validar cada redirección. Por eso el proceso solo conecta con un relay interno por trabajo, autenticado mediante una ruta aleatoria de 256 bits y enlazado exclusivamente a `127.0.0.1` en un puerto efímero. No es un servidor RPC, no acepta URLs en las peticiones y no se publica en la red.

El relay admite únicamente GET y HEAD con Host y ruta exactos. Rechaza cabeceras duplicadas, cuerpos, cookies, Authorization y Transfer-Encoding; limita las cabeceras a 8 KiB, su lectura a cinco segundos y las transferencias concurrentes a 16. HEAD devuelve los metadatos ya inspeccionados. Cada GET recupera únicamente el archivo aprobado, sin reenviar cabeceras de cuenta o navegador.

Las solicitudes externas usan reqwest con TLS verificado, DNS público comprobado y fijado para cada petición, y redirecciones manuales limitadas a cinco. Se rechazan destinos locales o reservados y descensos de HTTPS a HTTP. La inspección resuelve la cadena inicial; la transferencia posterior debe permanecer en su origen final o requiere una inspección nueva. Cada rango verifica su respuesta y el tamaño. Los enlaces firmados permanecen en el backend y no aparecen en la línea de comandos del proceso aria2. El relay no devuelve redirecciones al motor.

## Reanudación y checksum

Los controles `.aria2` y el estado propio del trabajo permanecen en su carpeta temporal al pausar, cancelar o fallar. Para reanudar se exige soporte de rangos y un ETag fuerte o un SHA-256 esperado de confianza. El estado guarda una huella de la URL, nombre seguro, tamaño, ETag y checksum esperado; no guarda la URL firmada. Si esos datos cambian, el parcial se conserva y el usuario prepara un trabajo nuevo para la nueva versión. Sin una identidad fiable de versión, no se mezclan parciales de distintas ejecuciones.

El SHA-256 opcional debe contener exactamente 64 caracteres hexadecimales. Debe provenir de una fuente que el usuario considere fiable. Un hash calculado después de descargar no demuestra por sí mismo autenticidad. Si el esperado no coincide, el trabajo falla y no publica el archivo en la biblioteca. Los archivos y temporales no se borran automáticamente.

El límite por archivo es 128 GiB y el de una ejecución es 24 horas. Un tamaño ausente se muestra como desconocido. El nombre procede de Content-Disposition o de la ruta final y se sanea para Windows, sin utilizar una shell. Los originales del usuario no se sobrescriben.

## Comprobación

Las pruebas de `advanced_files.rs` cubren respuestas de rango, límites, argumentos del motor, nombres y checksums. Los fixtures de red enlazan solo en loopback y no requieren instalar aria2. Un recorrido con el motor real queda como prueba ignorada que un desarrollador puede ejecutar con un binario oficial verificado. No se ha instalado ni ejecutado aria2 durante la implementación de este módulo.
