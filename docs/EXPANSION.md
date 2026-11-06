# Ampliación de fuentes y herramientas

El encargo se implementa sobre Tauri 2, Rust y TypeScript existentes. La referencia visual sigue siendo [design-reference.md](design-reference.md). Se conservan cola, sesiones protegidas, biblioteca, preparador de lotes y seguimiento existentes.

## Contratos y entregas

- A. Registro de operaciones por proveedor y catálogo conectado; gallery-dl opcional, exploración acotada, selección por adjunto, descarga por archivo y colección común.
- B. Proveedores de vídeo/audio, podcasts RSS con selección y OPML, directos limitados y detección de medios por acción explícita en la extensión.
- C. aria2 opcional, consentimiento P2P, metadatos antes de contenido, selección y publicación verificada en la cola.
- D. Telegram con credenciales propias, sesión protegida y navegación de medios autorizados.
- E. Adaptadores independientes de cursos, selección de materiales y estudio local.
- F. Herramientas locales y transcripción opcional, con salida en cola y biblioteca.

Un único integrador modifica cola, configuración y comandos compartidos. Los módulos independientes exponen sus contratos antes de integrarse. No se instalan componentes en el equipo durante el desarrollo, no se inician sesiones y no se descarga contenido personal.

Cada entrega debe pasar comprobación TypeScript, pruebas Rust relevantes y recorrido renderizado cuando afecte a la interfaz. Los commits usan mensajes breves en inglés y se suben al remoto existente por petición expresa del usuario. Ninguna cuenta sin probar se declara verificada. Una integración bloqueada no detiene las independientes.

## Estado

Primera entrega integrada: catálogo y explorador de galerías con selección por archivo, proveedores adicionales de vídeo/audio, RSS en Fuentes con OPML/M3U, selector BitTorrent con consentimiento, cursos locales y adaptadores de acceso autorizado, herramientas FFmpeg y transcripción opcional. Telegram, documentos y directos continúan en implementación.

Verificación del primer bloque: TypeScript, build Vite y comprobación de identidad correctos; 22 pruebas JavaScript y 165 pruebas Rust correctas. Otras 17 pruebas Rust requieren motores, contenido o servicios y permanecen marcadas como ignoradas. La prueba sintética de herramientas FFmpeg se ejecutó expresamente y pasó con entradas locales generadas. La suite nativa usa temporales del repositorio y necesita permiso para archivos y sockets de loopback, restringidos por el sandbox.

No se han iniciado sesiones, instalado componentes opcionales ni descargado archivos personales. Los extractores de galerías y las cuentas online requieren comprobación en vivo. Las capturas [Plataformas](expansion-platforms.png) y [Herramientas](expansion-tools.png) muestran el render de navegador, no una certificación de los servicios externos.

Consulta [PODCASTS.md](PODCASTS.md), [COURSES.md](COURSES.md), [TORRENTS.md](TORRENTS.md), [TRANSCRIPTION.md](TRANSCRIPTION.md) y [TOOLS.md](TOOLS.md) para los recorridos y límites.
