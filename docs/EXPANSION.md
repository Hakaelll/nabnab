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

Los seis bloques tienen módulos integrados en la cola y biblioteca existentes. Además del primer bloque se añadieron Telegram, directos con límite y parada guardando el resultado, descargas HTTP con aria2, documentos PDF/EPUB, exportaciones de fuentes, seguimiento de galerías, autenticación acotada de Pixiv/Mastodon, edición de subtítulos, unión de clips, sustitución de audio, paletas y preescucha de audio.

La interfaz conserva la identidad monocromática. La navegación llega al borde superior; los controles de herramientas se agrupan con separadores en lugar de tarjetas repetidas. Se eliminó el nombre bajo el emblema. Diez activaciones del símbolo alternan el multicolor, también con teclado. El modo normal recibe oleadas suaves de tonos vecinos. La reducción de movimiento desactiva las oleadas y el desplazamiento del color.

## Evidencia

- TypeScript, build Vite y comprobación de identidad correctos; 26 pruebas JavaScript y 224 pruebas Rust correctas. Otras 23 pruebas nativas requieren motores o servicios y están marcadas como ignoradas por defecto.
- Se ejecutaron expresamente las pruebas con archivos sintéticos reales de herramientas FFmpeg, PDF con MuPDF, galería HTML/hoja de contacto, unión y audio de vídeo, paletas, preescucha y descarga HTTP con aria2. Los originales se conservaron.
- El recorrido de navegador con fixtures IPC pasó a 1280×800, 1180×780 y 900×620, y con escalados 125 % y 150 %. Incluye navegación, formularios, biblioteca de 1000 elementos, foco, reducción de movimiento/transparencia y estados de descarga. No certifica servicios externos.
- La barra lateral y el easter egg se comprobaron además en la interfaz renderizada con puntero y teclado. Capturas actuales: [Inicio](interface-monochrome.png), [Multicolor](interface-color-egg.png), [Herramientas](interface-tools-refined.png). Las capturas expansion-* conservan el primer bloque como referencia anterior.

## Límites comprobables

No se han iniciado sesiones, instalado componentes en el sistema ni descargado archivos personales. Los motores utilizados en las pruebas están dentro del repositorio ignorado y se verificaron contra sus paquetes oficiales. No se declara verificado un proveedor que no se haya utilizado con una cuenta real.

Telegram contiene el cliente y sus controles de permisos, pero necesita una DLL TDLib compatible suministrada por separado; no se ofrece una descarga automática sin un paquete verificado. Las cuentas de cursos, Pixiv y Mastodon necesitan autorización propia y validación en vivo. Los directos no se han grabado contra un servicio real. La transcripción necesita un modelo multilingüe opcional, cuya descarga e inferencia no se han ejecutado durante estas pruebas. Los extractores externos pueden cambiar o rechazar fuentes; las limitaciones aparecen como errores, nunca como resultados fabricados.

Consulta los documentos de cada módulo en este directorio para los formatos, recorridos, permisos y límites concretos. La compilación de FFmpeg comprueba las capacidades requeridas por su receta; una instalación antigua puede necesitar actualizar ese motor para WebP e incrustación de subtítulos.
