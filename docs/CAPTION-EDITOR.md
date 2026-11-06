# Editar subtítulos

El editor del inspector trabaja sobre una pista local importada o generada. Cada operación crea una copia SRT mediante la cola y conserva la pista original.

Permite corregir texto, inicio y fin de un segmento; buscar y sustituir texto literal en toda la pista; desplazar todos los tiempos; y sincronizar con dos pares de tiempos. La sustitución distingue mayúsculas y minúsculas.

Para sincronizar, indica dos tiempos de la pista y los dos tiempos en que deberían aparecer. Se aplica una transformación lineal de desplazamiento y velocidad. Ambos pares deben avanzar en el tiempo. Si cualquier segmento quedaría con tiempo negativo, duración no positiva o un tiempo no finito, se rechaza toda la operación. Un desplazamiento negativo adelanta los subtítulos y también respeta ese límite.

El editor admite hasta 100.000 segmentos y 2 MiB de texto por pista. Calcula el tamaño de una sustitución antes de reservar memoria. Los segmentos aceptan saltos de línea, pero no líneas vacías que puedan convertirse en nuevas entradas SRT. Los tiempos se exportan con precisión de milisegundos. El texto se trata como texto plano; estilos y posiciones de la pista original no se reproducen en la copia.

Las pruebas cubren resultados exactos de edición, sustitución, desplazamiento y sincronización, rechazo de transformaciones inválidas y expansión de texto. Una prueba sobre archivos crea una copia SRT, la vuelve a leer y comprueba que el archivo del autor permanece intacto.
