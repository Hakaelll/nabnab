# Paletas de imágenes

La extracción de paletas acepta una imagen o una selección de biblioteca y genera un TXT con colores HEX o un archivo de variables CSS en la cola común. Conserva las imágenes originales y utiliza el componente FFmpeg existente, sin dependencias nuevas.

Cada imagen se reduce a un máximo de 128 × 128 píxeles RGBA. Las animaciones aportan su primer fotograma. Los píxeles completamente transparentes se ignoran; los parcialmente transparentes tienen un peso proporcional a su alfa. El agrupamiento ponderado obtiene hasta el máximo elegido de 2 a 16 colores representativos. Una imagen uniforme produce su único color disponible; no se añaden colores ficticios para completar el número.

El módulo permite entre 1 y 200 imágenes, con un máximo de 256 MiB por archivo. El archivo temporal de muestreo no supera 64 KiB por imagen. Los histogramas utilizan una cuantización de cinco bits por canal y el agrupamiento tiene un número de iteraciones acotado. Es una paleta de referencia obtenida por muestreo, no un inventario exacto de todos los píxeles del original.

La previsualización lee los colores del resultado TXT o CSS guardado en la biblioteca. No muestra muestras inventadas antes del procesamiento. Permite copiar la lista HEX o las variables CSS de esos colores reales.

Las pruebas unitarias usan valores RGB literales para comprobar colores dominantes, ponderación alfa, rechazo de muestras inválidas y formatos de salida. La prueba opcional con FFmpeg genera imágenes PNG de rojo, azul y verde transparente, comprueba los colores exportados y verifica que los originales no cambian.
