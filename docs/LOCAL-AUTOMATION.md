# Automatización local: CLI y MCP

`nabnab-cli.exe` envía solicitudes por IPC al propietario de la cola. Puede ser la aplicación abierta o el trabajador local iniciado expresamente con `nabnab-cli worker`. El repositorio admite un solo propietario. La CLI cliente conserva el estado y los resultados en la misma cola mediante ese servicio.

La automatización está desactivada inicialmente. El usuario activa el servicio, elige destinos y autoriza clientes en Ajustes → Automatización. Una conexión nueva solo registra una solicitud pendiente. Ninguna descarga o transformación se ejecuta hasta que su plan concreto se revise y apruebe en la interfaz local. El plan caduca a los diez minutos y su aprobación se consume una vez.

## Trabajador sin ventana

Activa Automatización, autoriza el cliente y aprueba el plan en Ajustes. Cierra nabnab y ejecuta `nabnab-cli worker` en una terminal. Mantén abierta esa terminal mientras envías `enqueue --plan PLAN_ID --client CLIENT_ID` desde otra. Ctrl+C cierra el trabajador mediante su canal de vida y la cola guarda el estado de los trabajos cancelados.

El trabajador admite únicamente planes específicos ya aprobados, vigentes y sin consumir. Las nuevas conexiones, las nuevas propuestas y los planes pendientes requieren aprobación en el escritorio. El trabajador utiliza el mismo bloqueo y las mismas comprobaciones de ejecución. Si otro propietario tiene abierto el repositorio, su inicio falla.

## Conexión y credenciales

```text
nabnab-cli connect --name "Mi cliente local" --json
nabnab-cli permissions --client CLIENT_ID --json
nabnab-cli status --client CLIENT_ID --json
```

`connect` devuelve un identificador público de cliente y el estado pendiente. La aplicación crea un secreto aleatorio de 256 bits, guardado bajo `automation-credentials` con ACL del usuario actual y cifrado DPAPI de Windows. El identificador tiene 32 caracteres hexadecimales; el secreto no se devuelve, imprime ni acepta como argumento. La CLI lo carga automáticamente para ese usuario. Copiar el archivo de credenciales a otro identificador no lo hace válido, porque DPAPI también utiliza ese identificador como contexto.

La comunicación usa una tubería con nombre de Windows, sin puerto TCP. El nombre incorpora SHA-256 de la ruta canónica de configuración y el SID del usuario. El servidor aplica una ACL del usuario actual, rechaza clientes remotos y reserva la primera instancia. Antes de enviar credenciales, la CLI verifica que el proceso servidor pertenece al mismo SID y abre el canal con nivel de identificación, sin permitir que el servidor suplante al cliente. Esto separa usuarios de Windows; no pretende aislar programas maliciosos que ya ejecuten código con acceso completo al mismo perfil de Windows.

El directorio se resuelve mediante `NABNAB_DATA_DIR` cuando está configurado, o `%APPDATA%/com.ytnabber.desktop`. Las compilaciones de depuración dan prioridad a `NABNAB_TEST_DATA_DIR` para pruebas aisladas. La CLI y el propietario de la cola deben usar el mismo directorio. No se admiten enlaces simbólicos ni junctions para las credenciales o la identidad del canal.

## Operaciones

```text
nabnab-cli inspect --url "https://example.org/public-media.mp4" --client CLIENT_ID --json
nabnab-cli search --query "cámara" --limit 20 --client CLIENT_ID --json
nabnab-cli get-item --item LIBRARY_ID --client CLIENT_ID --json
nabnab-cli jobs --client CLIENT_ID --json
nabnab-cli status --job JOB_ID --client CLIENT_ID --json
nabnab-cli cancel --job JOB_ID --client CLIENT_ID --json
```

Cada capacidad, colección y destino se concede por cliente. `permissions` devuelve sus destinos autorizados como identificadores y etiquetas, junto con sus colecciones, capacidades y presupuestos, sin requerir permiso de lectura de trabajos. La búsqueda devuelve metadatos de las colecciones concedidas. Los textos completos, notas, OCR y credenciales nunca forman parte de estos resultados. `includeDocuments` permite metadatos de documentos, artículos y subtítulos cuando se concede expresamente; no habilita lectura de su contenido. Los trabajos de otros clientes quedan fuera del alcance.

La inspección y las descargas se limitan a archivos multimedia directos públicos por HTTP(S), sin sesiones o cookies guardadas. No se siguen páginas web, extractores de plataformas ni listas HLS; esas funciones siguen requiriendo los flujos del escritorio. No se ofrecen captura de pantalla, micrófono, repetición, voz, shell, argumentos arbitrarios de motores ni rutas libres de entrada o salida. Las transformaciones aceptan identificadores de Biblioteca; los destinos, identificadores de carpetas concedidas previamente.

Un archivo JSON permite preparar una extracción de audio:

```json
{
  "operation": {"kind": "extractAudio", "libraryId": "LIBRARY_ID", "format": "mp3"},
  "outputRootId": "OUTPUT_ROOT_ID",
  "collection": "Revisión",
  "budget": {"maxBytes": 33554432, "maxSeconds": 60}
}
```

```text
nabnab-cli plan --params-file operation.json --client CLIENT_ID --json
nabnab-cli enqueue --plan APPROVED_PLAN_ID --client CLIENT_ID --json
```

`--params-json` también admite un objeto JSON literal. Nunca se interpreta como código. `plan` y `tools` guardan solicitudes pendientes; `tools` solo admite `extractAudio` o `frame`, y tampoco ejecuta nada directamente. Tras aprobar en el escritorio, `enqueue` solo acepta el identificador del plan: no permite cambiar la operación, entrada, destino, colección o presupuesto aprobado.

El catálogo inicial admite descarga pública en MP4, MP3, M4A, Opus o formato de origen; extracción local a MP3 o M4A; y fotograma de vídeo en un instante no negativo y ancho de 64–1920 píxeles. El presupuesto permitido es de 1–512 MiB y 5–600 segundos, dentro de los límites concedidos al cliente. La cola controla esos límites, la vigencia de permisos y la identidad de la entrada, y publica resultados mediante sus mecanismos existentes. Una aprobación no sustituye los permisos actuales del cliente.

## JSON y errores

El formato de salida estable de la CLI es versión 1. Con `--json` cada ejecución ordinaria devuelve un objeto compacto por stdout; sin esa opción se muestra el mismo objeto con sangría. Los errores también tienen un diagnóstico breve por stderr. La ayuda y la versión son excepciones de texto para personas.

```json
{"version":1,"ok":true,"result":{"jobs":[]}}
```

```json
{"version":1,"ok":false,"error":{"code":"approval_required","message":"Review and approve this exact plan in Settings → Automation.","retryable":false}}
```

Los códigos de salida son 0 para éxito, 2 para argumentos o formato inválidos, 3 cuando falta el propietario o la plataforma no está disponible, 4 para autorización o credenciales, 5 cuando falta aprobación del plan y 1 para otros errores. Un timeout de una mutación no prueba que no se haya guardado: comprueba el estado antes de volver a solicitarla. La aprobación del plan sigue siendo de un solo uso.

El canal interno usa JSON UTF-8 precedido por su longitud `u32` en little endian. Cada conexión transporta una solicitud y una respuesta. Se rechazan solicitudes de más de 256 KiB antes de reservar su cuerpo y las respuestas se limitan a 4 MiB. El servidor permite cuatro solicitudes activas, limita la lectura inicial a diez segundos, el manejador a 120 segundos y la escritura de respuesta a cinco segundos. Un cliente que deja de leer no retiene indefinidamente un permiso de conexión. Un acuse breve confirma que el cliente recibió la respuesta antes de cerrar la conexión. Este protocolo es local y no constituye un servicio HTTP público.

## MCP por stdio

```json
{
  "mcpServers": {
    "nabnab-local": {
      "command": "C:/ruta/instalada/nabnab-cli.exe",
      "args": ["mcp", "--client", "CLIENT_ID"]
    }
  }
}
```

Autoriza primero el cliente en el escritorio. `nabnab-cli mcp` comprueba su conexión antes de iniciar MCP. Después, stdout se reserva exclusivamente para JSON-RPC del SDK; los diagnósticos usan stderr. Cerrar la sesión stdio no cierra nabnab ni convierte la CLI en propietaria del repositorio. Desactivar el servicio o revocar permisos en el escritorio afecta a las siguientes llamadas y a los trabajos conforme a las comprobaciones de la cola.

El adaptador utiliza el [SDK oficial de Rust](https://github.com/modelcontextprotocol/rust-sdk), fijado a [rmcp 3.9.0](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.9.0), publicado el 5 de noviembre de 2026. Se activan únicamente servidor y transporte stdio; no se habilitan los transportes HTTP ni OAuth del SDK. La negociación de versión, inicialización, JSON-RPC y ciclo de sesión pertenecen al SDK, y los esquemas de las ocho herramientas son cerrados y concretos:

| Herramienta MCP | Método local |
| --- | --- |
| `media_inspect` | `media.inspect` |
| `jobs_plan` | `jobs.plan` |
| `jobs_enqueue` | `jobs.enqueue` |
| `jobs_status` | `jobs.status` |
| `jobs_cancel` | `jobs.cancel` |
| `library_search` | `library.search` |
| `library_get_item` | `library.get_item` |
| `tools_run` | `tools.run` |

Las anotaciones MCP describen las herramientas; los permisos se vuelven a comprobar en el escritorio y no dependen de que el cliente respete esas anotaciones. Los fallos operativos se devuelven como resultados de herramienta con `isError`, manteniendo el objeto de error versión 1. No se imprimen secretos en instrucciones de configuración.

## Verificación

El empaquetado existente de [Tauri CLI 2.12.4 compila todos los binarios Cargo y los descubre en `src/bin`](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-cli/src/interface/rust.rs). Su [generador NSIS incorpora cada binario que no sea el principal](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs). La plantilla de nabnab ya incluye instalación y eliminación de esos binarios. `default-run = "nabnab"` conserva el ejecutable principal; los módulos auxiliares de la CLI viven fuera de `src/bin` para que no se confundan con ejecutables adicionales. El instalador coloca `nabnab-cli.exe` junto a `nabnab.exe`, sin modificar el PATH del sistema.

Tras generar el instalador, `node scripts/verify-cli-package.mjs RENDERED_INSTALLER_NSI RELEASE_CLI_EXE` comprueba el script NSIS realmente renderizado, sus instrucciones de instalación y eliminación, el subsistema de consola del PE y la salida de `--version`. Esta comprobación no ejecuta el instalador y no sustituye una prueba de instalación.

Las pruebas de `automation_wire`, `automation_credentials` y `nabnab-cli` comprueban el encuadre acotado, los campos desconocidos, las respuestas coherentes, opciones cerradas, datos JSON literales, comparación de secretos e identidades DPAPI. Las pruebas nativas de tubería, CLI y negociación MCP deben ejecutarse con acceso normal a los componentes de seguridad de Windows, fuera del sandbox si este bloquea DPAPI o tuberías. No requieren una elevación de administrador de Windows.

Los recorridos integrados usan exclusivamente datos sintéticos en directorios `artifacts/automation` aislados. Sus resultados y las limitaciones observadas se registran cuando se ejecutan; una comprobación estática o una compilación no acreditan el recorrido completo.

La ejecución nativa de tests/native-automation.browser.mjs completó cuatro grupos sin errores. Usó la tubería real, la CLI y la negociación stdio del SDK, generó un MP3 y un PNG mediante la cola, y rechazó la salida fuera de presupuesto. Con la aplicación cerrada, el trabajador produjo un M4A a partir de un plan aprobado y rechazó una conexión y un plan que aún requerían aprobación. Se precargaron una carpeta de salida y colecciones sintéticas. La autorización de clientes y planes se ejecutó en el backend real. [Resultados](verification/automation-native.json).
