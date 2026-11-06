# BitTorrent opcional

El módulo usa aria2 como proceso local. No inicia un servidor RPC, no añade trackers, no configura el firewall o el router y no ejecuta archivos recibidos. La instalación del componente y la activación de P2P son decisiones separadas.

## Uso

1. Instala el componente opcional Archivos/BitTorrent desde Ajustes → Componentes.
2. En el formulario BitTorrent, lee y acepta el aviso sobre peers y subida de piezas antes de activar el módulo.
3. Abre un archivo `.torrent` local o pega un magnet v1. Abrir el archivo local solo lee metadatos. Obtener metadatos de un magnet conecta con peers y tiene un límite de dos minutos y una acción para detenerlo.
4. Revisa el árbol de archivos, sus tamaños y el destino. La selección inicial está vacía. Selecciona archivos o carpetas y configura límites de subida y bajada. `0` significa sin límite.
5. Añade la selección a Descargas. Pausa y reintenta desde la cola común. Los archivos parciales y los controles de aria2 permiten reanudar; aria2 verifica los hashes de las piezas.

La política inicial utiliza `--seed-time=0`, que detiene el seeding después de completar la descarga. No elimina la subida de piezas mientras descarga. Las políticas opcionales permiten un tiempo de 1 a 1440 minutos o un ratio entre 0,1 y 20, con un límite adicional de 24 horas para el ratio. Cuando se especifican ratio y tiempo, aria2 termina cuando se cumple cualquiera de ellos. Estas opciones siguen el [manual oficial de aria2 1.37.0](https://aria2.github.io/manual/en/html/aria2c.html).

## Compatibilidad y límites

Se admiten metadatos BitTorrent v1 y magnets `urn:btih` de 40 caracteres hexadecimales o 32 caracteres Base32. No se anuncian BitTorrent v2, torrents híbridos, web seeds ni parámetros magnet que incorporen otras fuentes. Los trackers admitidos son HTTP, HTTPS y UDP públicos, sin credenciales en la URL. Los torrents privados conservan su indicador y aria2 aplica sus restricciones de DHT y PEX.

Los nombres deben ser UTF-8 y válidos en Windows. Se rechazan enlaces simbólicos, rutas que salen del destino, nombres reservados, duplicados que solo difieren por mayúsculas y metadatos de más de 16 MiB. El límite es de 50 000 archivos. Los metadatos seleccionados se conservan en un directorio administrado y la cola utiliza una referencia de integridad, sin devolver rutas internas al formulario.

La descarga parcial puede recibir piezas que contienen bytes de archivos no seleccionados. Los temporales no se eliminan automáticamente para evitar perder la reanudación. Un archivo solo cuenta como salida cuando aria2 termina correctamente y su ruta y tamaño corresponden al manifiesto. El árbol muestra el orden del torrent; no asigna prioridades por archivo que aria2 no ofrece.

BitTorrent comunica la dirección IP a peers y trackers. El módulo no ofrece anonimato. La validación de nombres de trackers no equivale a aislar todas las conexiones P2P. Las sesiones y cookies web de nabnab no se envían a peers ni a trackers.

## Verificación

Las pruebas de `torrents.rs` cubren lectura de manifiestos simples y multifichero, orden y tamaño, rechazo de rutas peligrosas, magnets v1, límites de selección, argumentos del motor y lectura de progreso y seeding. Se ejecutan con `cargo test --manifest-path src-tauri/Cargo.toml torrents::tests`.

La transferencia en vivo requiere el componente aria2 y un torrent autorizado con peers disponibles. No se ha ejecutado una transferencia P2P durante esta implementación ni se ha activado el módulo en el equipo del usuario.
