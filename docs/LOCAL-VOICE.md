# Voz local de textos elegidos

La voz local genera WAV a partir de un texto, una versión de artículo, una nota o texto extraíble de un documento elegido. La extracción y la relación con Biblioteca pertenecen al host de nabnab; el motor recibe ese texto y su identidad de versión. No graba pantalla, micrófono ni audio del sistema.

## Motor y voces

En Windows se utiliza `System.Speech.Synthesis.SpeechSynthesizer`, incluido en .NET Framework, sobre voces SAPI de escritorio ya instaladas. Se enumeran las voces reales y sus idiomas; no se incluyen identificadores inventados ni modelos descargados por defecto. La selección se limita a voces cuyo proveedor, token y nombre reportados corresponden a Microsoft Desktop SAPI. Otros motores de terceros podrían tener requisitos de red y quedan fuera de este adaptador.

El programa ejecuta un script fijo con Windows PowerShell, sin perfiles ni ventana de consola. El texto y las opciones viajan en un JSON privado; no se interpolan en código ni en argumentos de shell. Se llama a `Speak(String)` para texto corriente, no al método de SSML. La salida se dirige siempre a un archivo WAV: preparar, mostrar una muestra o generar no activa altavoces automáticamente. La reproducción es una acción explícita de la interfaz.

No se usa Edge TTS, Azure Speech ni otro servicio remoto como alternativa. El adaptador no instala voces, cambia el Registro ni copia binarios del sistema. El motor y las voces instalados conservan sus términos de Windows; nabnab no redistribuye modelos ni anuncia una licencia abierta para esos recursos. La calidad y la pronunciación dependen de la voz y del idioma elegidos; no hay clonación de voces ni doblaje automático.

Las APIs oficiales utilizadas son [SpeechSynthesizer](https://learn.microsoft.com/en-us/dotnet/api/system.speech.synthesis.speechsynthesizer?view=netframework-4.8.1), [InstalledVoice](https://learn.microsoft.com/en-us/dotnet/api/system.speech.synthesis.installedvoice?view=netframework-4.8.1), [VoiceInfo.AdditionalInfo](https://learn.microsoft.com/en-us/dotnet/api/system.speech.synthesis.voiceinfo.additionalinfo?view=netframework-4.8.1) y [salida WAV con formato explícito](https://learn.microsoft.com/en-us/dotnet/api/system.speech.synthesis.speechsynthesizer.setoutputtowavefile?view=netframework-4.8.1). Estas referencias describen motores instalados y el control de idioma, velocidad, volumen y salida; no acreditan la disponibilidad de una voz en un equipo concreto.

Si no hay una voz compatible, la función lo informa. Las voces heredadas se gestionan manualmente en Windows → Hora e idioma → Voz. La documentación de Microsoft explica cómo [añadir voces y qué idiomas ofrece Windows](https://support.microsoft.com/es-es/accessibility/windows/narrator/appendix-a-supported-languages-and-voices). No todas las voces de Narrador o las voces naturales son accesibles mediante System.Speech: solo se ofrecen las que este motor enumera como compatibles. La instalación y su descarga requieren la decisión del usuario en Windows.

## Muestra, segmentos y reanudación

El usuario elige una voz instalada, velocidad de −10 a 10 y volumen de 0 a 100. El idioma procede de la voz real. La muestra usa un fragmento inicial de hasta 200 caracteres Unicode y devuelve como máximo treinta segundos de audio. El resultado informa si el límite de treinta segundos recortó la muestra.

Un plan conserva título y origen, identificador relacionado de Biblioteca cuando existe, SHA-256 del texto, identidad SHA-256 de la versión de origen, configuración de voz y huella de motor. Las fronteras de segmento son posiciones reales en puntos de código Unicode del texto guardado, no posiciones UTF-8 ni marcas inventadas por palabra. Se conservan los párrafos y se dividen los fragmentos largos en límites de frase o palabra cuando es posible, con hasta 480 caracteres por segmento.

Un solo proceso del motor genera los segmentos pendientes. Cada WAV se cierra y se valida antes de publicarlo en la caché privada. La interfaz puede consultar qué segmentos terminaron y reproducirlos mientras continúa la generación. Pausar o cancelar detiene el proceso y conserva los segmentos completos; el segmento que no llegó a publicarse puede necesitar generarse otra vez. Reanudar verifica su integridad antes de reutilizarlo.

La identidad de caché incluye el texto exacto del segmento, voz, velocidad, volumen, huella de motor y versión del adaptador/formato. La huella utiliza las versiones de System.Speech/SAPI/Windows y los atributos de voz que Windows reporta, incluidos los de `AdditionalInfo`. Esta API no ofrece un manifiesto criptográfico de todos los archivos internos del modelo; no se afirma esa certificación. Si cambia la huella reportada del motor o de la voz, se exige preparar un plan nuevo. Los audios ya guardados siguen siendo resultados de su versión anterior.

## Salida y límites

Los segmentos usan PCM mono de 16 bits a 22.050 Hz. El WAV final se escribe por bloques a partir de los segmentos comprobados, sin cargar el audio completo en memoria. Se crea un archivo nuevo, con nombre derivado del título y una identidad de plan, sin sobrescribir resultados existentes. El host puede usar después las conversiones de audio existentes de nabnab.

Los capítulos siguen los segmentos sintetizados. Su comienzo, final y duración se calculan a partir del número real de muestras PCM. Los títulos muestran un índice y hasta veinte caracteres de contexto; el texto completo permanece en el plan. Así, incluso 4.096 títulos Unicode caben en el registro de publicación de Biblioteca. Solo se resaltan segmentos completos que estén reproduciéndose; no se prometen posiciones exactas de palabras.

Cada plan admite hasta un millón de caracteres Unicode, 4 MiB de texto y 4.096 segmentos. Cada segmento WAV está limitado a 16 MiB y la suma de datos PCM de un resultado a 3 GiB, dentro del límite RIFF de 32 bits. El texto no determina una duración ni un tamaño exactos: la voz y su velocidad cambian el resultado. Los segmentos conservados para reanudar también ocupan espacio local, además del WAV final.

Los planes y el audio intermedio se guardan bajo `voice-plans` y `voice-cache`, en directorios privados de la configuración local. Pueden contener texto y voz de documentos privados elegidos por el usuario. No forman parte de un servicio público. El cliente solo transmite identidades opacas y números de segmento; las rutas de los archivos temporales y de caché se construyen dentro del backend.

## Contratos para el host

- `status(root, cancel)` enumera disponibilidad y voces locales. El host añade el estado de activación del componente opcional, separado del estado de cada voz instalada.
- `prepare(root, text, sourceVersion, origin, settings, cancel)` devuelve un `VoicePlan` persistente. `Origin` se construye en el host a partir de la selección autorizada; el cliente no introduce una ruta.
- `read_plan(root, planId)` valida la versión, identidad, texto y fronteras guardadas. El plan incluye nombre e idioma de la voz para conservar procedencia en Biblioteca.
- `plan_status(root, planId)` devuelve índices, disponibilidad, duraciones y bytes reales de segmentos terminados. No retransmite el texto por cada consulta ni relee todo el audio: usa recibos, tamaño y encabezados WAV como información de estado.
- `segment_audio(root, planId, index)` valida el hash completo antes de devolver un segmento WAV acotado. `sample` devuelve bytes WAV, texto de muestra, duración real e indicador de recorte.
- `execute(root, planId, staging, cancel, runId, emit)` integra cancelación y progreso comunes, y devuelve el archivo nuevo y sus capítulos. El plan persistente conserva la configuración y la procedencia que el host relaciona con Biblioteca. Los hashes completos se verifican antes de reutilizar audio y antes de componer el resultado final.

Los comandos del host y la cola conservan la relación con el archivo y la versión de texto originales, requieren activación explícita del componente y no mantienen bloqueado el repositorio mientras trabaja el motor.

## Verificación

Las pruebas unitarias comprueban límites de texto/libro, fronteras Unicode, identidad de planes y configuración, rechazo de cambios del texto guardado, integridad de audio en caché, estado compacto, WAV inválidos o truncados, muestra acotada, concatenación, capítulos con duraciones reales y preservación de la caché al cancelar.

La prueba ignorada `voice::tests::real_desktop_voice_sample_partial_cancel_resume_and_large_plan` requiere `NABNAB_TEST_VOICE=1` y una voz Microsoft Desktop ya instalada. Genera únicamente textos sintéticos a WAV; no reproduce altavoces. Comprueba muestra, preparación de más de 100.000 caracteres sin síntesis masiva, disponibilidad de un segmento antes del resultado final, cancelación, reanudación y conservación de bytes de segmentos completos. Debe ejecutarse fuera del sandbox cuando las restricciones de acceso a componentes del sistema impidan enumerar las voces; eso no implica necesitar permisos de administrador de Windows.

La ejecución Rust del 6 de noviembre de 2026 pasó cinco pruebas unitarias y la prueba nativa completa. Generó una muestra con Microsoft Helena Desktop `es-ES` de 4,648 segundos y 205.006 bytes, y otra con Microsoft Zira Desktop `en-US` de 5,793 segundos y 255.534 bytes, ambas con versión de voz reportada 11.0. También comprobó un plan de más de 100.000 caracteres, lectura de los bytes WAV de un segmento terminado antes del resultado final, cancelación, reanudación sin cambiar sus bytes y duración final coherente con los capítulos. Los registros están en `artifacts/voice/unit-tests.log` y `artifacts/voice/native-tests.log`.

Esta evidencia describe las voces presentes en el equipo de prueba; no garantiza que estén instaladas en otra máquina. No se activaron altavoces ni se descargaron voces. El recorrido visual integrado se comprueba por separado.

El recorrido integrado de tests/native-voice.browser.mjs también pasó cuatro grupos en la aplicación nativa con datos aislados: muestras reales Helena/Zira sin reproducción, 40 segmentos y pausa/reanudación desde caché, extracción de artículo/nota/TXT/PDF/EPUB y cinco WAV relacionados con sus fuentes. Las notas ancladas a una versión distinta se rechazan. La Biblioteca muestra seis narraciones y conserva procedencia, configuración y duraciones reales. Los originales mantuvieron sus hashes; no hubo peticiones a proveedores remotos. Resultados: [voice-native.json](verification/voice-native.json).
