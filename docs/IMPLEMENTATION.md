# Arquitectura de nabnab

nabnab conserva Tauri 2, Rust, Vite y TypeScript sin framework UI. No añade navegador, reproductor pesado ni motor 3D. La interfaz invoca casos de uso nativos; no construye comandos ni escribe la persistencia.

## Recorrido

Inicio analiza un enlace con debounce y cancelación. Cada solicitud tiene un identificador creciente; tanto Rust como la interfaz descartan respuestas antiguas. El análisis nunca inicia una descarga. La selección de un elemento o lote crea trabajos persistidos antes de que el planificador pueda ejecutarlos.

`providers.rs` identifica proveedores mediante hosts parseados y devuelve capacidades. `model.rs` transforma JSON de yt-dlp en metadatos acotados; no interpreta títulos como HTML. Los resúmenes de listas no inventan calidad ni pistas que todavía no se han analizado. La matriz de plataformas separa contratos de resultados reales.

`direct.rs` trata los archivos HTTP/HTTPS genéricos con un cliente nativo: valida y fija DNS en cada salto, acota redirecciones y tamaño, y reanuda únicamente con ETag fuerte y rangos coherentes. La cola deja comprobar primero los recibos locales completos para recuperar procesamiento sin conexión. El extractor genérico de yt-dlp permanece deshabilitado. No se aceptan manifiestos remotos arbitrarios. Véase [HTTP directo](DIRECT.md).

`queue.rs` es el único planificador de descargas y transformaciones. Reserva cada ejecución bajo el bloqueo del repositorio, registra su cancelación antes de soltarlo y aplica límites globales, por proveedor y de procesamiento. Los alias de una plataforma comparten límite. Los eventos incluyen trabajo y ejecución; un evento anterior no cambia la ejecución actual. El progreso frecuente vive en memoria; las transiciones se guardan en disco.

`engine.rs` ejecuta binarios mediante argumentos estructurados sin shell. La distribución resuelve rutas administradas y no depende del PATH del desarrollador. Un Windows Job Object termina FFmpeg, Deno y los demás descendientes cuando se cancela o cierra el proceso. yt-dlp ignora configuraciones y plugins externos; Deno tiene una ruta explícita y se deshabilitan componentes remotos.

`media.rs` comprueba códecs, contenedor, duración y tamaño con ffprobe. MP4 compatible recodifica solamente las pistas incompatibles. Los perfiles originales conservan los códecs y utilizan el contenedor real. Los archivos locales se analizan con protocolos y demuxers restringidos para rechazar manifiestos disfrazados de vídeo.

## Archivos y recuperación

`store.rs` mantiene configuración, cola y biblioteca en un repositorio JSON versionado. Una instancia conserva un bloqueo exclusivo. Las escrituras se validan, se sincronizan y se publican con reemplazo atómico. La migración de la antigua configuración conserva una copia y no mueve medios. Véase [migración](MIGRATION.md).

Cada trabajo escribe en un directorio temporal propio dentro de su destino, con marcador de propiedad. Antes de publicar, la cola guarda un recibo con metadatos y SHA-256. El movimiento final nunca reemplaza un archivo existente. Si hay un corte después del movimiento y antes de guardar el estado, un reintento explícito puede adoptar el resultado cuya huella coincide. Los trabajos interrumpidos se recuperan pausados; no se inicia actividad inesperada.

La publicación reutiliza una salida con los mismos bytes, perfil y destino después de comprobar de nuevo su huella real. Un bloqueo asíncrono abarca la decisión final, el movimiento y el registro de trabajo/biblioteca para que dos terminaciones concurrentes no publiquen duplicados. El análisis, la transferencia y la conversión ocurren antes de ese bloqueo. Esta reutilización evita otro archivo final; no promete evitar una transferencia anterior.

`library.rs` indexa rutas autorizadas sin copiar archivos y conserva colecciones, etiquetas y relaciones entre originales y resultados. Importar carpetas utiliza límites de recorrido y rechaza enlaces y junctions. Quitar un registro no elimina el medio. Localizar un archivo conserva su identidad y organización.

`tools.rs` prepara convertir, extraer audio, recortar y comprimir. Todas las operaciones usan la cola, no sobrescriben el original y verifican sus salidas. La conversión muestra progreso indeterminado cuando el motor no proporciona una medida real. No se anuncia aceleración de hardware sin detección.

## Reproducción y miniaturas

`local_media.rs` sirve únicamente identificadores de archivos ya autorizados por la biblioteca. No acepta rutas del sistema enviadas por la vista. Soporta rangos acotados de hasta 4 MiB para reproducción sin cargar un vídeo entero en memoria. La UI utiliza la reproducción de WebView2 y ofrece el reproductor de Windows cuando el formato no es compatible.

`support.rs` genera miniaturas locales de forma diferida, con ejecución acotada y caché limitada a 512 archivos y aproximadamente 128 MiB. La limpieza solo incluye miniaturas reconocidas del directorio con marcador propio. Los diagnósticos excluyen medios, rutas de biblioteca, títulos, sesiones y tokens.

## Integraciones opcionales

`sessions.rs` usa DPAPI y ACL privadas para sesiones por sitio. Los archivos temporales se eliminan mediante un guard que vive hasta terminar el proceso. `dependencies.rs` administra conjuntos verificados de motores y rollback; el actualizador de yt-dlp sigue siendo una operación separada. `preferences.rs` aplica bandeja y registro de inicio opcional de Windows, desactivados inicialmente.

`browser.rs` recibe solicitudes de extensiones emparejadas en loopback. Valida Host, Origin, token, esquema, tamaño e identificador de solicitud. La extensión no extrae multimedia ni lee sesiones del navegador. La [política de red](NETWORK.md) y el [contrato de extensión](BROWSER.md) describen límites y destinos.

La interfaz se reparte entre el controlador `src/main.ts`, contratos y vistas por área. Conserva selección, filtros, formulario, posición y foco al navegar o recibir progreso. La biblioteca pagina 60 registros; las miniaturas se solicitan al entrar en la zona visible. El renderer original se limita a Inicio, se detiene al ocultarse y respeta movimiento reducido.

## Identidad y distribución

La marca visible, ejecutable y artefactos son `nabnab`. El identificador `com.ytnabber.desktop` y las claves históricas de NSIS conservan continuidad. La plantilla del instalador detecta el ejecutable anterior y migra únicamente accesos directos que apuntaban a él. No se inventan editor, certificado, servicio de actualización ni dominio.

La evidencia está en [verificación](NABNAB-VERIFICATION.md). Las capturas con IPC simulado, las pruebas nativas, los motores reales y las mediciones se registran por separado.
