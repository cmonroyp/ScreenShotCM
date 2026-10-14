# ScreenShotCM

**Capturas de pantalla y anotación para Windows 10 y 11.** Pulsa Impr Pant,
elige un área y la captura ya está en tu portapapeles, lista para anotar.
Gratis, sin cuenta, sin publicidad y sin conexión a internet.

![El editor de ScreenShotCM con una captura anotada](https://cmonroyp.github.io/screenshotcm-web/assets/img/es/background.png)

**[Descargar para Windows](https://github.com/cmonroyp/screenshotcm-web/releases/latest/download/ScreenShotCM.msi)** ·
[Sitio web](https://cmonroyp.github.io/screenshotcm-web/) ·
[Privacidad](https://cmonroyp.github.io/screenshotcm-web/privacy.html) ·
[Soporte](https://github.com/cmonroyp/screenshotcm-web/issues) ·
[English](#english)

## Instalar

Descarga el instalador y ábrelo. Se instala para tu usuario, sin pedir
administrador, y queda en la bandeja del sistema, arrancando con Windows. Una
versión nueva se instala encima de la anterior. Se desinstala desde
*Configuración → Aplicaciones*; tus capturas y ajustes se conservan.

El instalador todavía no va firmado: Windows SmartScreen puede avisar al
abrirlo (*Más información → Ejecutar de todas formas*). Cada versión, en
[Releases](https://github.com/cmonroyp/screenshotcm-web/releases), publica su
SHA-256 para comprobar que el archivo es el original.

## Qué hace

- **Capturar** un área, una ventana, la pantalla completa o la misma área
  otra vez, también con temporizador. La pantalla se congela mientras
  seleccionas, con lupa, cruz y medidas.
- **Anotar** en el editor, que se abre al capturar: flechas rectas o curvas,
  rectángulos, elipses, líneas, texto, contadores numerados, foco, lápiz,
  resaltador, y pixelado o desenfoque para ocultar datos. Recorte, deshacer y
  zoom.
- **Presentar** la captura sobre un fondo degradado, con margen, esquinas
  redondeadas y sombra.
- **Compartir**: cada captura va al portapapeles y aparece en una miniatura
  para copiarla, guardarla o arrastrarla a otra aplicación. Historial del
  último mes.
- **Fijar** una captura sobre todas las ventanas como referencia.
- **Ligera**: menos de 2 MB de memoria en reposo y un único ejecutable.
- En **español e inglés**, según el idioma de Windows o el que elijas.

## Atajos

Por defecto (se cambian en *Ajustes → Atajos de teclado*):

| | |
|---|---|
| Capturar área | Impr Pant |
| Capturar ventana | Shift+Impr Pant (o Espacio durante la selección) |
| Pantalla completa | Ctrl+Impr Pant |
| Área anterior | Alt+Shift+Impr Pant |

Durante la selección: Shift la hace cuadrada, las flechas mueven el puntero
píxel a píxel, Esc o clic derecho cancelan. En el editor: V R F O L A T B S N P
H C eligen la herramienta (pulsar otra vez la suelta), Ctrl+Z / Ctrl+Y
deshacen y rehacen, Ctrl+C copia, Ctrl+S guarda, Ctrl+rueda hace zoom.

## Automatizar

Desde la línea de comandos, un acceso directo o un enlace:

```
screenshotcm.exe --capture-area
screenshotcm.exe --pin C:\captura.png
screenshotcm.exe C:\imagen.png          (la abre en el editor)
screenshotcm://capture-window
```

Órdenes: `capture-area`, `capture-previous-area`, `capture-fullscreen`,
`capture-window`, `self-timer`, `open-from-clipboard`, `open-file`,
`open-annotate`, `pin`, `open-history`, `restore-recently-closed`,
`unlock-pins`, `close-pins`, `open-save-folder`, `open-settings`,
`reload-settings`, `quit`. Un enlace `screenshotcm://` solo acepta archivos en
una unidad local de tu equipo.

## Privacidad y seguridad

ScreenShotCM no se conecta a internet, no tiene telemetría y no escucha en
ningún puerto. Tus capturas y ajustes se quedan en tu equipo
(`%LOCALAPPDATA%\ScreenShotCM` y `%APPDATA%\ScreenShotCM`). Ver la
[política de privacidad](https://cmonroyp.github.io/screenshotcm-web/privacy.html).

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

- **[Download for Windows](https://github.com/cmonroyp/screenshotcm-web/releases/latest/download/ScreenShotCM.msi)**
  — installs for your user, no administrator needed, and starts with Windows.
  The installer is not signed yet, so SmartScreen may warn you (*More info →
  Run anyway*); each release publishes its SHA-256.
- Capture an area, a window or the full screen; annotate with curved arrows,
  text, counters, pixelate and more; present it on a gradient; copy, save,
  drag or pin it. Under 2 MB of memory at rest. English and Spanish.
- Default shortcuts: Print Screen (area), Shift+Print Screen (window),
  Ctrl+Print Screen (full screen), Alt+Shift+Print Screen (previous area).
- Privacy: nothing leaves your PC — [privacy policy](https://cmonroyp.github.io/screenshotcm-web/privacy.html).
  Report vulnerabilities privately: [SECURITY.md](SECURITY.md).
- Licence: [GPL-3.0-or-later](LICENSE); the name and icon are reserved
  ([TRADEMARKS.md](TRADEMARKS.md)). Building: [DEVELOPMENT.md](DEVELOPMENT.md).
