# Transcripción local

Herramientas → Voz y subtítulos → Transcribir usa whisper.cpp en CPU y la cola común. Requiere instalar y activar el componente opcional y descargar un modelo desde esa pantalla. No instala ni descarga modelos durante el arranque.

1. Instala whisper.cpp desde Ajustes → Componentes.
2. En Transcribir, elige Tiny o Small y pulsa su botón de descarga. La pantalla muestra tamaño, procedencia, idiomas y licencia antes de descargar.
3. Selecciona vídeo o audio de la biblioteca, idioma y un máximo de 1–8 hilos CPU. El valor efectivo no supera los procesadores disponibles.
4. Añade la tarea a la cola. Se crea un SRT nuevo con origen Automático y relación con el archivo original. La biblioteca permite buscar sus frases y exportar TXT, SRT o VTT mediante el lector existente.

El contenido se genera automáticamente y puede ser inexacto. El resultado nunca sustituye pistas del autor. No se envía audio a servicios externos. Esta implementación no anuncia aceleración GPU; ejecuta `--no-gpu` y limita hilos CPU.

## Componentes y procedencia

El ejecutable es el componente opcional whisper.cpp. La interfaz del CLI se ha contrastado con el [CLI oficial v1.7.6](https://github.com/ggml-org/whisper.cpp/blob/v1.7.6/examples/cli/cli.cpp). FFmpeg convierte la pista de audio seleccionada a WAV mono de 16 kHz en el directorio de trabajo de la tarea, sin modificar el original. El proceso se cancela mediante el mismo canal y control de procesos de las otras tareas.

Los modelos proceden del [repositorio de modelos de ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp). Ambos son multilingües, incluyen español y tienen licencia MIT. Los archivos se verifican por tamaño y SHA-256 durante la descarga y antes de cada transcripción.

| Modelo | Tamaño exacto | SHA-256 |
| --- | ---: | --- |
| Tiny | 77.691.713 bytes | `be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21` |
| Small | 487.601.967 bytes | `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` |

Las referencias originales son los punteros publicados de [Tiny](https://huggingface.co/ggerganov/whisper.cpp/raw/main/ggml-tiny.bin) y [Small](https://huggingface.co/ggerganov/whisper.cpp/raw/main/ggml-small.bin). Los modelos `.en` no se ofrecen como valor predeterminado ni se presentan como multilingües.

No se ha ejecutado reconocimiento real durante esta implementación porque requeriría instalar el motor y descargar un modelo. Las pruebas locales cubren opciones, límites y argumentos; el recorrido con voz debe comprobarse después de instalar voluntariamente el componente y un modelo.
