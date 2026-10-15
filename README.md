# ScreenShotCM

**Capturas de pantalla y anotación para Windows 10 y 11.** Pulsa Impr Pant,
elige un área y la captura ya está en tu portapapeles, lista para anotar.
Gratis, sin cuenta, sin publicidad y sin conexión a internet.

![El editor de ScreenShotCM con una captura anotada](https://screenshotcm.github.io/assets/img/es/background.png)

<a href="https://apps.microsoft.com/detail/9P0HHRD6G2QL?mode=direct"><img src="https://get.microsoft.com/images/es%20dark.svg" alt="Obtener en Microsoft Store" width="200"></a>

**[Microsoft Store](https://apps.microsoft.com/detail/9P0HHRD6G2QL)** ·
[Instalador MSI](https://github.com/screenshotcm/screenshotcm.github.io/releases/latest/download/ScreenShotCM.msi) ·
[Sitio web](https://screenshotcm.github.io/) ·
[Privacidad](https://screenshotcm.github.io/privacy.html) ·
[Soporte](https://github.com/screenshotcm/screenshotcm.github.io/issues) ·
[English](#english)

## Instalar

**Desde la [Microsoft Store](https://apps.microsoft.com/detail/9P0HHRD6G2QL)** (recomendado): un clic, firmado por
Microsoft y con actualizaciones automáticas.

**O con el instalador MSI**: descárgalo y ábrelo. Se instala para tu usuario,
sin pedir administrador. El MSI no va firmado: Windows SmartScreen puede avisar
al abrirlo (*Más información → Ejecutar de todas formas*). Cada versión, en
[Releases](https://github.com/screenshotcm/screenshotcm.github.io/releases), publica su
SHA-256 para comprobar que el archivo es el original.

De las dos formas queda en la bandeja del sistema y arranca con Windows. Se
desinstala desde *Configuración → Aplicaciones*; tus capturas y ajustes se
conservan.

## Qué hace

- **Capturar** un área, una ventana, la pantalla completa o la misma área
  otra vez, también con temporizador. La pantalla se congela mientras
  seleccionas, con lupa, cruz y medidas.
- **Captura con desplazamiento**: páginas, chats y documentos más largos que
  la pantalla, en una sola imagen. Elige el área y desplázate con la rueda, o
  pulsa *Desplazar solo* y la app baja hasta el final por ti; una vista previa
  muestra cómo crece. Los encabezados fijos aparecen una sola vez.
- **Anotar** en el editor, que se abre al capturar: flechas rectas o curvas,
  rectángulos, elipses, líneas, texto, contadores numerados, foco, lápiz,
  resaltador (el lápiz y el resaltador se enderezan solos cuando el trazo va
  casi recto). Para ocultar datos: pixelar, desenfocar (seguro o suave) o
  tapar en negro.
- **Recortar y transformar**: recorte libre o con proporción (cuadrado, 4:3,
  3:2, 16:9, 9:16 u original) que se ajusta a los bordes de la imagen; girar y
  voltear; cambio de tamaño en píxeles o en %, sin deformar; *Volver al
  original* deshace todo eso de una vez. Deshacer sin límite y zoom.
- **Presentar** la captura sobre un fondo degradado, con margen, esquinas
  redondeadas y sombra. El último fondo elegido se aplica a la siguiente
  captura; *Sin fondo* lo quita.
- **Compartir**: cada captura va al portapapeles y aparece en una miniatura
  para copiarla, guardarla o arrastrarla a otra aplicación. Nada se guarda
  en el disco si no lo pides: el historial de capturas es opcional
  (*Ajustes → Historial*).
- **Fijar** una captura sobre todas las ventanas como referencia.
- **Ligera**: menos de 2 MB de memoria en reposo y un único ejecutable.
- En **español e inglés**, en **modo claro u oscuro**: los de Windows o los
  que elijas en *Ajustes*.

## Atajos

Por defecto (se cambian en *Ajustes → Atajos de teclado*):

| | |
|---|---|
| Capturar área | Impr Pant |
| Capturar ventana | Shift+Impr Pant (o Espacio durante la selección) |
| Pantalla completa | Ctrl+Impr Pant |
| Área anterior | Alt+Shift+Impr Pant |
| Captura con desplazamiento | Ctrl+Shift+Impr Pant |

Durante la selección: Shift la hace cuadrada, las flechas mueven el puntero
píxel a píxel, Esc o clic derecho cancelan. Durante una captura con
desplazamiento: Intro la termina y Esc la cancela. En el editor: V R F O L A T B S N P
H C eligen la herramienta (pulsar otra vez la suelta), Ctrl+Z / Ctrl+Y
deshacen y rehacen, Ctrl+C copia, Ctrl+S guarda. Ctrl+rueda, Ctrl + y Ctrl −
acercan y alejan; Ctrl+1 muestra el tamaño real y Ctrl+0 ajusta a la ventana.
Una imagen más grande que la ventana se arrastra con la mano: con la flecha
sobre una zona vacía, manteniendo Espacio con cualquier herramienta, o con el
botón central del ratón; la rueda la recorre también. El tamaño de la imagen,
abajo, abre el menú para cambiarlo: escribe el ancho y pulsa Intro, o elige un
porcentaje. Un texto se mueve arrastrándolo, o por su borde punteado mientras
lo escribes; un clic en ese borde (o Esc) lo selecciona y Supr lo borra entero.

## Automatizar

Desde la línea de comandos, un acceso directo o un enlace:

```
screenshotcm.exe --capture-area
screenshotcm.exe --pin C:\captura.png
screenshotcm.exe C:\imagen.png          (la abre en el editor)
screenshotcm://capture-window
```

Órdenes: `capture-area`, `capture-previous-area`, `capture-fullscreen`,
`capture-window`, `capture-scrolling`, `self-timer`, `open-from-clipboard`, `open-file`,
`open-annotate`, `pin`, `open-history`, `restore-recently-closed`,
`unlock-pins`, `close-pins`, `open-save-folder`, `open-settings`,
`reload-settings`, `quit`. Un enlace `screenshotcm://` solo acepta archivos en
una unidad local de tu equipo.

## Privacidad y seguridad

ScreenShotCM no se conecta a internet, no tiene telemetría y no escucha en
ningún puerto. Tus capturas y ajustes se quedan en tu equipo
(`%LOCALAPPDATA%\ScreenShotCM` y `%APPDATA%\ScreenShotCM`). Ver la
[política de privacidad](https://screenshotcm.github.io/privacy.html).

Si encuentras una vulnerabilidad, repórtala en privado como explica
[SECURITY.md](SECURITY.md), nunca en un issue público.

## Licencia

Software libre bajo la [GNU General Public License v3.0 o posterior](LICENSE):
puedes usarlo, estudiarlo, modificarlo y redistribuirlo, y cualquier versión
que distribuyas debe publicarse también con su código bajo la misma licencia.
El nombre «ScreenShotCM» y su icono no están cubiertos: una versión modificada
debe llevar otro nombre y otro icono ([TRADEMARKS.md](TRADEMARKS.md)).

Para compilarlo o contribuir, ver [DEVELOPMENT.md](DEVELOPMENT.md).

---

## English

**Screenshots and annotation for Windows 10 and 11.** Press Print Screen, pick
an area, and the capture is already on your clipboard, ready to annotate.
Free, no account, no ads, no internet connection.

- **[Get it from the Microsoft Store](https://apps.microsoft.com/detail/9P0HHRD6G2QL)** (recommended): one click,
  signed by Microsoft, updated automatically.
- Or the **[MSI installer](https://github.com/screenshotcm/screenshotcm.github.io/releases/latest/download/ScreenShotCM.msi)**:
  installs for your user, no administrator needed, and starts with Windows.
  The MSI is not signed, so SmartScreen may warn you (*More info → Run
  anyway*); each release publishes its SHA-256.
- Capture an area, a window or the full screen, or a whole page, chat or
  document by scrolling (by hand or automatically); annotate with curved
  arrows, text, counters, pixelate and more; present it on a gradient; copy,
  save, drag or pin it. Under 2 MB of memory at rest. English and Spanish.
- Default shortcuts: Print Screen (area), Shift+Print Screen (window),
  Ctrl+Print Screen (full screen), Alt+Shift+Print Screen (previous area),
  Ctrl+Shift+Print Screen (scrolling capture).
- Privacy: nothing leaves your PC — [privacy policy](https://screenshotcm.github.io/privacy.html).
  Report vulnerabilities privately: [SECURITY.md](SECURITY.md).
- Licence: [GPL-3.0-or-later](LICENSE); the name and icon are reserved
  ([TRADEMARKS.md](TRADEMARKS.md)). Building: [DEVELOPMENT.md](DEVELOPMENT.md).
